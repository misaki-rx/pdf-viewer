#[allow(dead_code)]
#[path = "../tests/support/page_streams.rs"]
mod fixtures;

use hayro::hayro_syntax::Pdf;
use std::{hint::black_box, sync::Arc, time::Instant};

fn median(samples: &mut [f64]) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

fn main() {
    let passes: usize = std::env::args()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(5);
    assert!((1..=31).contains(&passes));
    let stress = fixtures::compressed(128, 1024 * 1024);
    if let Some(output) = std::env::args_os().nth(2) {
        std::fs::write(output, &stress).unwrap();
    }
    println!(
        "{passes} passes; cold sequential decoding, then cached last-page hits and first-page revisit. Medians exclude fixture generation, PDF parsing, final release, interpretation, rasterization, GPU and UI. Retained bytes sum capacities of live decoded-page snapshots after dropping all caller-owned strong references."
    );
    for (name, bytes) in [
        (
            "guide",
            include_bytes!("../assets/pagewing-guide.pdf").as_slice(),
        ),
        (
            "128 independently compressed 1 MiB pages",
            stress.as_slice(),
        ),
    ] {
        let mut scans = Vec::new();
        let mut hits = Vec::new();
        let mut revisits = Vec::new();
        let mut retained = 0;
        let mut decoded = 0;
        let mut resident = 0;
        for _ in 0..passes {
            let pdf = Pdf::new(bytes.to_vec()).unwrap();
            let mut snapshots = Vec::with_capacity(pdf.pages().len());
            let mut total = 0;
            let start = Instant::now();
            for page in pdf.pages().iter() {
                let stream = page.page_stream().unwrap();
                total += stream.len();
                snapshots.push(Arc::downgrade(&stream));
                black_box(stream);
            }
            scans.push(start.elapsed().as_secs_f64() * 1000.0);
            let mut capacity = 0;
            let mut count = 0;
            for snapshot in &snapshots {
                if let Some(stream) = snapshot.upgrade() {
                    capacity += stream.capacity();
                    count += 1;
                }
            }
            assert!(capacity <= 32 * 1024 * 1024 && count <= 512);
            retained = capacity;
            resident = count;
            decoded = total;
            let last = pdf.pages().last().unwrap();
            let start = Instant::now();
            for _ in 0..20_000 {
                black_box(last.page_stream().unwrap());
            }
            hits.push(start.elapsed().as_secs_f64() * 1e9 / 20_000.0);
            let start = Instant::now();
            black_box(pdf.pages()[0].page_stream().unwrap());
            revisits.push(start.elapsed().as_secs_f64() * 1000.0);
            drop(pdf);
            assert!(
                snapshots
                    .iter()
                    .all(|snapshot| snapshot.upgrade().is_none())
            );
        }
        println!(
            "{name}: file {} bytes, decoded {:.2} MiB, retained {:.2} MiB / {resident} pages; scan {:.3} ms, hit {:.1} ns, revisit {:.3} ms",
            bytes.len(),
            decoded as f64 / 1048576.0,
            retained as f64 / 1048576.0,
            median(&mut scans),
            median(&mut hits),
            median(&mut revisits),
        );
    }
}
