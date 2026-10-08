//! Usage: build-dict <ecdict.csv> <output.pkd>
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: build-dict <ecdict.csv> <output.pkd>");
        std::process::exit(2);
    }
    let file = std::fs::File::open(&args[1]).unwrap_or_else(|e| {
        eprintln!("Cannot open {}: {e}", args[1]);
        std::process::exit(1)
    });
    let entries = peek_dict::entries_from_csv(std::io::BufReader::new(file)).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1)
    });
    let bytes = peek_dict::build(entries);
    let output = std::path::Path::new(&args[2]);
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    let result = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(&bytes)?;
        file.as_file().sync_all()?;
        file.persist(output).map_err(|e| e.error)?;
        Ok(())
    })();
    result.unwrap_or_else(|e| {
        eprintln!("Cannot publish {}: {e}", args[2]);
        std::process::exit(1)
    });
    println!("Wrote {} bytes to {}", bytes.len(), args[2]);
}
