//! cargo run --release -p peek-dict --example bench -- local-assets/ecdict.pkd
use std::time::Instant;

fn main() {
    let path = std::env::args().nth(1).expect("path to .pkd");
    let t = Instant::now();
    let dict = peek_dict::Dict::open(std::path::Path::new(&path)).expect("open");
    println!("load: {:?} ({} entries)", t.elapsed(), dict.len());
    let words = ["stream", "given", "apple", "ubiquitous", "qzxqzx"];
    let t = Instant::now();
    let rounds = 10_000;
    for _ in 0..rounds {
        for w in words {
            std::hint::black_box(dict.lookup(w));
        }
    }
    println!(
        "lookup: {:?} per query",
        t.elapsed() / (rounds * words.len() as u32)
    );
}
