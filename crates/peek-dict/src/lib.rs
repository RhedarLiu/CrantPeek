//! Offline dictionary: compact sorted binary index built from ECDICT (MIT) CSV.
//!
//! PKD2: magic | count | index byte length | delta-varint offsets | entry data.
//! Entries contain five varint-length-prefixed UTF-8 fields. Offsets are expanded
//! once at open, preserving constant-time random access and binary search.
//! PKD1 dictionaries remain readable. Every read is bounds-checked.

const MAGIC: &[u8; 4] = b"PKD2";
const LEGACY_MAGIC: &[u8; 4] = b"PKD1";

fn write_varint(mut value: u32, out: &mut Vec<u8>) {
    while value >= 128 {
        out.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn read_varint(bytes: &[u8], position: &mut usize) -> Option<u32> {
    let mut value = 0u32;
    for shift in (0..35).step_by(7) {
        let byte = *bytes.get(*position)?;
        *position += 1;
        if shift == 28 && byte > 15 {
            return None;
        }
        value |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Not a Crant Peek dictionary file")]
    BadFormat,
    #[error("Dictionary file is truncated or corrupted")]
    Corrupt,
    #[error("CSV error: {0}")]
    Csv(#[from] csv::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Entry {
    pub word: String,
    pub phonetic: String,
    /// One meaning per line, as stored by ECDICT.
    pub translation: String,
    pub pos: String,
    /// ECDICT exchange field, e.g. `p:gave/d:given/0:give`.
    pub exchange: String,
}

impl Entry {
    /// Lemma ("0:" item of the exchange field), if this entry is an inflected form.
    pub fn lemma(&self) -> Option<&str> {
        self.exchange
            .split('/')
            .find_map(|item| item.strip_prefix("0:"))
            .filter(|l| !l.is_empty())
    }
    /// Human-readable word forms, e.g. [("form-past","gave"), …]; the lemma item is excluded.
    pub fn forms(&self) -> Vec<(&'static str, &str)> {
        self.exchange
            .split('/')
            .filter_map(|item| {
                let (kind, value) = item.split_once(':')?;
                let label = match kind {
                    "p" => "form-past",
                    "d" => "form-past-participle",
                    "i" => "form-present-participle",
                    "3" => "form-third-person",
                    "r" => "form-comparative",
                    "t" => "form-superlative",
                    "s" => "form-plural",
                    _ => return None,
                };
                (!value.is_empty()).then_some((label, value))
            })
            .collect()
    }
}

fn key(word: &str) -> String {
    word.trim().to_lowercase()
}

/// Serialise entries (duplicates by lowercase word keep the first occurrence).
pub fn build(mut entries: Vec<Entry>) -> Vec<u8> {
    entries.retain(|e| !key(&e.word).is_empty());
    entries.sort_by_cached_key(|e| key(&e.word));
    entries.dedup_by(|a, b| key(&a.word) == key(&b.word));
    let mut index = Vec::new();
    let mut data = Vec::new();
    let mut previous = 0;
    for e in &entries {
        let offset = u32::try_from(data.len()).expect("dictionary exceeds 4GiB");
        write_varint(offset - previous, &mut index);
        previous = offset;
        for field in [&e.word, &e.phonetic, &e.translation, &e.pos, &e.exchange] {
            write_varint(
                u32::try_from(field.len()).expect("field exceeds 4GiB"),
                &mut data,
            );
            data.extend_from_slice(field.as_bytes());
        }
    }
    let mut out = Vec::with_capacity(12 + index.len() + data.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    out.extend_from_slice(&(index.len() as u32).to_le_bytes());
    out.extend_from_slice(&index);
    out.extend_from_slice(&data);
    out
}

/// Parse ECDICT CSV. Entries without a Chinese translation are dropped to keep the file small.
pub fn entries_from_csv(reader: impl std::io::Read) -> Result<Vec<Entry>, Error> {
    let mut csv = csv::ReaderBuilder::new().flexible(true).from_reader(reader);
    let headers = csv.headers()?.clone();
    let col = |name: &str| headers.iter().position(|h| h == name);
    let (Some(word), Some(translation)) = (col("word"), col("translation")) else {
        return Err(Error::BadFormat);
    };
    let (phonetic, pos, exchange) = (col("phonetic"), col("pos"), col("exchange"));
    let (bnc, frq) = (col("bnc"), col("frq"));
    let mut out = Vec::new();
    for record in csv.records() {
        let record = record?;
        let get = |i: Option<usize>| i.and_then(|i| record.get(i)).unwrap_or("").to_owned();
        let rank = |i: Option<usize>| {
            i.and_then(|i| record.get(i))
                .and_then(|v| v.trim().parse::<u32>().ok())
                .filter(|v| *v > 0)
        };
        let entry = Entry {
            word: get(Some(word)),
            // ECDICT encodes line breaks as the two characters backslash + n.
            translation: get(Some(translation))
                .replace("\\n", "\n")
                .replace("\\r", ""),
            phonetic: get(phonetic),
            pos: get(pos),
            exchange: get(exchange),
        };
        if entry.translation.trim().is_empty() {
            continue;
        }
        // Keep the file small: single words always stay; multi-word phrases and
        // hyphenated/odd tokens only when they appear in a frequency list.
        let simple = entry
            .word
            .chars()
            .all(|c| c.is_alphabetic() || c == '\'' || c == '-');
        if !simple && rank(bnc).is_none() && rank(frq).is_none() {
            continue;
        }
        out.push(entry);
    }
    Ok(out)
}

enum Storage {
    Owned(Vec<u8>),
    Mapped(memmap2::Mmap),
}
impl std::ops::Deref for Storage {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            Self::Owned(bytes) => bytes,
            Self::Mapped(map) => map,
        }
    }
}

pub struct Dict {
    bytes: Storage,
    count: usize,
    data_start: usize,
    compact_offsets: Option<Vec<u32>>,
}

impl Dict {
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, Error> {
        Self::from_storage(Storage::Owned(bytes))
    }
    fn from_storage(bytes: Storage) -> Result<Self, Error> {
        if bytes.len() < 8 {
            return Err(Error::Corrupt);
        }
        let count = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        if &bytes[..4] == LEGACY_MAGIC {
            let data_start = count
                .checked_mul(4)
                .and_then(|n| n.checked_add(8))
                .ok_or(Error::Corrupt)?;
            if bytes.len() < data_start {
                return Err(Error::Corrupt);
            }
            return Ok(Self {
                bytes,
                count,
                data_start,
                compact_offsets: None,
            });
        }
        if &bytes[..4] != MAGIC {
            return Err(Error::BadFormat);
        }
        let index_len =
            u32::from_le_bytes(bytes.get(8..12).ok_or(Error::Corrupt)?.try_into().unwrap())
                as usize;
        let data_start = 12usize.checked_add(index_len).ok_or(Error::Corrupt)?;
        let index = bytes.get(12..data_start).ok_or(Error::Corrupt)?;
        if count > index_len {
            return Err(Error::Corrupt);
        }
        let mut offsets = Vec::with_capacity(count);
        let mut position = 0;
        let mut offset = 0u32;
        for i in 0..count {
            let delta = read_varint(index, &mut position).ok_or(Error::Corrupt)?;
            if (i == 0 && delta != 0) || (i != 0 && delta == 0) {
                return Err(Error::Corrupt);
            }
            offset = offset.checked_add(delta).ok_or(Error::Corrupt)?;
            if offset as usize >= bytes.len() - data_start {
                return Err(Error::Corrupt);
            }
            offsets.push(offset);
        }
        if position != index.len() {
            return Err(Error::Corrupt);
        }
        Ok(Self {
            bytes,
            count,
            data_start,
            compact_offsets: Some(offsets),
        })
    }

    pub fn open(path: &std::path::Path) -> Result<Self, Error> {
        let file = std::fs::File::open(path).map_err(|_| Error::BadFormat)?;
        // Dictionary files must be immutable while open. Builder/update tools publish a new
        // file rather than truncating an existing mapping. Read-only mapping avoids a full copy.
        let map =
            unsafe { memmap2::MmapOptions::new().map(&file) }.map_err(|_| Error::BadFormat)?;
        Self::from_storage(Storage::Mapped(map))
    }

    pub fn len(&self) -> usize {
        self.count
    }
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    fn offset_at(&self, index: usize) -> Option<usize> {
        if index >= self.count {
            return None;
        }
        if let Some(offsets) = &self.compact_offsets {
            Some(*offsets.get(index)? as usize)
        } else {
            let table = 8 + index.checked_mul(4)?;
            Some(u32::from_le_bytes(self.bytes.get(table..table + 4)?.try_into().ok()?) as usize)
        }
    }
    fn field_at<'a>(&self, record: &'a [u8], position: &mut usize) -> Option<&'a str> {
        let len = if self.compact_offsets.is_some() {
            read_varint(record, position)? as usize
        } else {
            let len = u32::from_le_bytes(
                record
                    .get(*position..position.checked_add(4)?)?
                    .try_into()
                    .ok()?,
            ) as usize;
            *position += 4;
            len
        };
        let end = position.checked_add(len)?;
        let text = std::str::from_utf8(record.get(*position..end)?).ok()?;
        *position = end;
        Some(text)
    }
    fn record_at(&self, index: usize) -> Option<&[u8]> {
        let start = self.data_start.checked_add(self.offset_at(index)?)?;
        let end = if index + 1 < self.count {
            self.data_start.checked_add(self.offset_at(index + 1)?)?
        } else {
            self.bytes.len()
        };
        self.bytes.get(start..end)
    }
    fn word_at(&self, index: usize) -> Option<&str> {
        self.field_at(self.record_at(index)?, &mut 0)
    }
    fn entry_at(&self, index: usize) -> Option<Entry> {
        let record = self.record_at(index)?;
        let mut position = 0;
        let mut fields: [String; 5] = Default::default();
        for field in &mut fields {
            *field = self.field_at(record, &mut position)?.to_owned();
        }
        let [word, phonetic, translation, pos, exchange] = fields;
        Some(Entry {
            word,
            phonetic,
            translation,
            pos,
            exchange,
        })
    }

    fn find(&self, word: &str) -> Option<Entry> {
        let wanted = key(word);
        let (mut lo, mut hi) = (0, self.count);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let candidate = self.word_at(mid)?;
            let order = if candidate.is_ascii() {
                candidate
                    .trim()
                    .bytes()
                    .map(|b| b.to_ascii_lowercase())
                    .cmp(wanted.bytes())
            } else {
                // String lowercasing includes context-dependent mappings (e.g. final sigma).
                key(candidate).cmp(&wanted)
            };
            match order {
                std::cmp::Ordering::Equal => return self.entry_at(mid),
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
            }
        }
        None
    }

    /// Look up a word; falls back to surrounding-punctuation stripping and one lemma hop
    /// (e.g. "gave" has no translation of its own in some data sets → "give").
    pub fn lookup(&self, word: &str) -> Option<Entry> {
        let trimmed = word.trim_matches(|c: char| !c.is_alphanumeric());
        let found = self.find(word).or_else(|| self.find(trimmed))?;
        if found.translation.trim().is_empty()
            && let Some(lemma) = found.lemma().and_then(|l| self.find(l))
        {
            return Some(lemma);
        }
        Some(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Dict {
        let e = |w: &str, t: &str, x: &str| Entry {
            word: w.into(),
            translation: t.into(),
            exchange: x.into(),
            phonetic: "p".into(),
            pos: "v:100".into(),
        };
        Dict::from_bytes(build(vec![
            e("Zebra", "斑马", ""),
            e("give", "给予", "p:gave/d:given/i:giving/3:gives"),
            e("gave", "", "0:give"),
            e("apple", "苹果", "s:apples"),
            e("APPLE", "重复", ""),
        ]))
        .unwrap()
    }

    fn legacy_bytes(entries: &[Entry]) -> Vec<u8> {
        let mut offsets = Vec::new();
        let mut data = Vec::new();
        for entry in entries {
            offsets.push(data.len() as u32);
            for field in [
                &entry.word,
                &entry.phonetic,
                &entry.translation,
                &entry.pos,
                &entry.exchange,
            ] {
                data.extend_from_slice(&(field.len() as u32).to_le_bytes());
                data.extend_from_slice(field.as_bytes());
            }
        }
        let mut bytes = b"PKD1".to_vec();
        bytes.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        for offset in offsets {
            bytes.extend_from_slice(&offset.to_le_bytes());
        }
        bytes.extend_from_slice(&data);
        bytes
    }

    #[test]
    fn compact_and_legacy_preserve_all_fields_and_search_results() {
        let entries = vec![
            Entry {
                word: "apple".into(),
                phonetic: "æpəl".into(),
                translation: "苹果\n".repeat(128),
                pos: "n:100".into(),
                exchange: "s:apples".into(),
            },
            Entry {
                word: "zebra".into(),
                translation: "斑马".into(),
                ..Default::default()
            },
        ];
        let legacy = legacy_bytes(&entries);
        let compact = build(entries.clone());
        assert!(compact.len() < legacy.len());
        let old = Dict::from_bytes(legacy).unwrap();
        let new = Dict::from_bytes(compact).unwrap();
        for (i, expected) in entries.iter().enumerate() {
            assert_eq!(old.entry_at(i).as_ref(), Some(expected));
            assert_eq!(new.entry_at(i).as_ref(), Some(expected));
        }
        for word in ["APPLE", "zebra", "apples", "missing"] {
            assert_eq!(old.lookup(word), new.lookup(word));
        }
    }

    #[test]
    fn corrupt_compact_indexes_are_rejected() {
        for index in [
            vec![0x80; 5],
            vec![0xff, 0xff, 0xff, 0xff, 0x10],
            vec![1],
            vec![0, 0],
        ] {
            let mut bytes = b"PKD2".to_vec();
            bytes.extend_from_slice(&1u32.to_le_bytes());
            bytes.extend_from_slice(&(index.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&index);
            bytes.extend_from_slice(&[0; 5]);
            assert!(Dict::from_bytes(bytes).is_err());
        }
        let empty = Dict::from_bytes(build(vec![])).unwrap();
        assert!(empty.is_empty());
        assert!(empty.lookup("a").is_none());
    }

    #[test]
    fn unicode_lowercase_search_matches_builder_sorting() {
        let dict = Dict::from_bytes(build(vec![
            Entry {
                word: "ΟΣ".into(),
                translation: "test".into(),
                ..Default::default()
            },
            Entry {
                word: "ÉCOLE".into(),
                translation: "school".into(),
                ..Default::default()
            },
        ]))
        .unwrap();
        assert_eq!(dict.lookup("ος").unwrap().word, "ΟΣ");
        assert_eq!(dict.lookup("école").unwrap().translation, "school");
    }
    #[test]
    fn mapped_lookup_matches_owned_and_invalid_files_rejected() {
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        let bytes = build(vec![Entry {
            word: "stream".into(),
            translation: "流".into(),
            ..Default::default()
        }]);
        file.write_all(&bytes).unwrap();
        file.as_file().sync_all().unwrap();
        let mapped = Dict::open(file.path()).unwrap();
        let owned = Dict::from_bytes(bytes).unwrap();
        assert_eq!(mapped.lookup("STREAM"), owned.lookup("STREAM"));
        assert_eq!(mapped.len(), owned.len());
        let empty = tempfile::NamedTempFile::new().unwrap();
        assert!(Dict::open(empty.path()).is_err());
        let mut bad = tempfile::NamedTempFile::new().unwrap();
        bad.write_all(b"bad dictionary").unwrap();
        assert!(Dict::open(bad.path()).is_err());
    }
    #[test]
    fn case_insensitive_lookup_and_dedup() {
        let d = sample();
        assert_eq!(d.len(), 4);
        assert_eq!(d.lookup("APPLE").unwrap().translation, "苹果");
        assert_eq!(d.lookup("zebra").unwrap().word, "Zebra");
        assert!(d.lookup("missing").is_none());
    }
    #[test]
    fn strips_punctuation_and_follows_lemma() {
        let d = sample();
        assert_eq!(d.lookup("\"apple,\"").unwrap().word, "apple");
        assert_eq!(d.lookup("gave").unwrap().word, "give");
    }
    #[test]
    fn word_forms() {
        let forms = sample().lookup("give").unwrap().forms().len();
        assert_eq!(forms, 4);
    }
    #[test]
    fn corrupted_files_never_panic() {
        let good = build(vec![Entry {
            word: "a".into(),
            translation: "一".into(),
            ..Default::default()
        }]);
        assert!(Dict::from_bytes(b"nope".to_vec()).is_err());
        for cut in 0..good.len() {
            if let Ok(d) = Dict::from_bytes(good[..cut].to_vec()) {
                let _ = d.lookup("a");
            }
        }
        let mut bad = good.clone();
        let n = bad.len();
        bad[n - 3] = 0xff; // corrupt a length prefix region
        if let Ok(d) = Dict::from_bytes(bad) {
            let _ = d.lookup("a");
        }
        let mut huge = good;
        huge[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Dict::from_bytes(huge).is_err());
    }
    #[test]
    fn unescapes_literal_newlines_and_filters_phrases() {
        let csv = "word,phonetic,definition,translation,pos,bnc,frq,exchange\n\
                   run,r,,n. 跑\\nv. 运行,,0,0,\n\
                   obscure phrase here,,,[网络] 生僻,,0,0,\n\
                   common phrase,,,常见短语,,500,0,\n";
        let entries = entries_from_csv(csv.as_bytes()).unwrap();
        let words: Vec<_> = entries.iter().map(|e| e.word.as_str()).collect();
        assert_eq!(words, ["run", "common phrase"]);
        assert_eq!(entries[0].translation, "n. 跑\nv. 运行");
    }
    /// Runs only when a locally downloaded ECDICT file exists (never in CI).
    #[test]
    fn real_ecdict_smoke() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../local-assets/ecdict/ecdict.csv");
        let Ok(file) = std::fs::File::open(path) else {
            return;
        };
        let dict = Dict::from_bytes(build(
            entries_from_csv(std::io::BufReader::new(file)).unwrap(),
        ))
        .unwrap();
        assert!(dict.len() > 100_000);
        let stream = dict.lookup("stream").expect("stream");
        assert!(stream.translation.contains('流'));
        assert!(!stream.translation.contains("\\n"));
        // Real data gives inflected forms their own entry; the lemma is exposed separately.
        let given = dict.lookup("given").expect("given");
        assert_eq!(given.word.to_lowercase(), "given");
        assert_eq!(given.lemma(), Some("give"));
        assert!(dict.lookup("give").is_some());
        assert!(dict.lookup("qzxqzxqzx").is_none());
    }
    #[test]
    fn parses_ecdict_csv() {
        let csv = "word,phonetic,definition,translation,pos,exchange\n\
                   run,rʌn,,\"跑\n运行\",v:80,p:ran/0:\n\
                   empty,x,,,,\n";
        let entries = entries_from_csv(csv.as_bytes()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].translation, "跑\n运行");
    }
}
