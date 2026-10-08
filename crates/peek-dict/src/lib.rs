//! Offline dictionary: compact sorted binary index built from ECDICT (MIT) CSV.
//!
//! Layout: b"PKD1" | u32 count | count * u32 entry offsets (relative to data start) | data.
//! Entry: 5 length-prefixed (u32 LE) UTF-8 fields: word, phonetic, translation, pos, exchange.
//! Entries are sorted by lowercase word, so lookup is a binary search. Every read is
//! bounds-checked: a truncated or corrupted file yields errors/None, never a panic.

const MAGIC: &[u8; 4] = b"PKD1";

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
    /// Human-readable word forms, e.g. [("过去式","gave"), …]; the lemma item is excluded.
    pub fn forms(&self) -> Vec<(&'static str, &str)> {
        self.exchange
            .split('/')
            .filter_map(|item| {
                let (kind, value) = item.split_once(':')?;
                let label = match kind {
                    "p" => "过去式",
                    "d" => "过去分词",
                    "i" => "现在分词",
                    "3" => "第三人称单数",
                    "r" => "比较级",
                    "t" => "最高级",
                    "s" => "复数",
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
    let mut offsets = Vec::with_capacity(entries.len());
    let mut data = Vec::new();
    for e in &entries {
        offsets.push(data.len() as u32);
        for field in [&e.word, &e.phonetic, &e.translation, &e.pos, &e.exchange] {
            data.extend_from_slice(&(field.len() as u32).to_le_bytes());
            data.extend_from_slice(field.as_bytes());
        }
    }
    let mut out = Vec::with_capacity(8 + offsets.len() * 4 + data.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for o in offsets {
        out.extend_from_slice(&o.to_le_bytes());
    }
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
}

impl Dict {
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, Error> {
        Self::from_storage(Storage::Owned(bytes))
    }
    fn from_storage(bytes: Storage) -> Result<Self, Error> {
        if bytes.len() < 8 || &bytes[..4] != MAGIC {
            return Err(Error::BadFormat);
        }
        let count = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        let table_end = count
            .checked_mul(4)
            .and_then(|n| n.checked_add(8))
            .ok_or(Error::Corrupt)?;
        if bytes.len() < table_end {
            return Err(Error::Corrupt);
        }
        Ok(Self { bytes, count })
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

    fn word_at(&self, index: usize) -> Option<&str> {
        let table = 8 + index.checked_mul(4)?;
        let offset =
            u32::from_le_bytes(self.bytes.get(table..table + 4)?.try_into().ok()?) as usize;
        let start = (8 + self.count * 4).checked_add(offset)?;
        let len = u32::from_le_bytes(self.bytes.get(start..start + 4)?.try_into().ok()?) as usize;
        std::str::from_utf8(self.bytes.get(start + 4..(start + 4).checked_add(len)?)?).ok()
    }
    fn entry_at(&self, index: usize) -> Option<Entry> {
        let table = 8 + index.checked_mul(4)?;
        let offset =
            u32::from_le_bytes(self.bytes.get(table..table + 4)?.try_into().ok()?) as usize;
        let data_start = 8 + self.count * 4;
        let mut pos = data_start.checked_add(offset)?;
        let mut fields: [String; 5] = Default::default();
        for field in &mut fields {
            let len = u32::from_le_bytes(self.bytes.get(pos..pos + 4)?.try_into().ok()?) as usize;
            pos += 4;
            let end = pos.checked_add(len)?;
            *field = std::str::from_utf8(self.bytes.get(pos..end)?)
                .ok()?
                .to_owned();
            pos = end;
        }
        let [word, phonetic, translation, pos_tag, exchange] = fields;
        Some(Entry {
            word,
            phonetic,
            translation,
            pos: pos_tag,
            exchange,
        })
    }

    fn find(&self, word: &str) -> Option<Entry> {
        let wanted = key(word);
        let (mut lo, mut hi) = (0, self.count);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let candidate = self.word_at(mid)?;
            match candidate
                .trim()
                .chars()
                .flat_map(char::to_lowercase)
                .cmp(wanted.chars())
            {
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
