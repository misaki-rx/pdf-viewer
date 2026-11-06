use super::*;
use hayro::{hayro_interpret::InterpreterCache, hayro_syntax::content::TypedIter};
use std::sync::Arc;

#[path = "../../tests/support/page_streams.rs"]
mod fixtures;

const MIB: usize = 1024 * 1024;

#[test]
fn scanning_compressed_pages_keeps_only_32_mib_and_redecodes_evicted_pages() {
    let bytes = fixtures::compressed(64, MIB);
    assert!(bytes.len() < 128 * 1024);
    let pdf = Pdf::new(bytes).unwrap();
    let first = pdf.pages()[0].page_stream().unwrap();
    let first_weak = Arc::downgrade(&first);
    let interpreter = InterpreterCache::new();
    let before = crate::text::extract(&pdf.pages()[0], &interpreter);
    let render_cache = RenderCache::new();
    let key = RenderKey {
        page: 0,
        width: 300,
        rotation: 0,
        night: false,
        tile: None,
    };
    let pixels = rasterize(&pdf, &render_cache, key).unwrap();
    drop(first);

    let mut snapshots = Vec::new();
    for (page, data) in pdf.pages().iter().enumerate() {
        // The application reaches the same cache through the interpreter.
        let text = crate::text::extract(data, &interpreter);
        assert_eq!(text.text, format!("Page {page}"));
        let stream = data.page_stream().unwrap();
        assert_eq!(stream.len(), MIB);
        snapshots.push(Arc::downgrade(&stream));
    }
    assert!(first_weak.upgrade().is_none());
    let retained: usize = snapshots
        .iter()
        .filter_map(|weak| weak.upgrade())
        .map(|stream| stream.capacity())
        .sum();
    assert!(retained <= 32 * MIB, "retained {retained} bytes");
    assert!(retained > 0);
    assert!(snapshots.last().unwrap().upgrade().is_some());

    let after = crate::text::extract(&pdf.pages()[0], &interpreter);
    assert_eq!(before.text, after.text);
    assert_eq!(before.spans.len(), after.spans.len());
    for (a, b) in before.spans.iter().zip(&after.spans) {
        assert_eq!(a.range, b.range);
        assert_eq!(a.rect, b.rect);
    }
    assert_eq!(
        rasterize(&pdf, &render_cache, key).unwrap().rgba,
        pixels.rgba
    );
    drop(render_cache);
    drop(interpreter);
    drop(pdf);
    assert!(snapshots.iter().all(|weak| weak.upgrade().is_none()));
}

#[test]
fn a_stream_snapshot_survives_eviction_and_the_entire_document() {
    let pdf = Pdf::new(fixtures::compressed(35, MIB)).unwrap();
    let first = pdf.pages()[0].page_stream().unwrap();
    for page in &pdf.pages()[1..] {
        drop(page.page_stream());
    }
    let redecoded = pdf.pages()[0].page_stream().unwrap();
    assert!(!Arc::ptr_eq(&first, &redecoded));
    assert_eq!(first, redecoded);
    drop(pdf);
    let mut operations = TypedIter::new(&first);
    assert!(operations.next().is_some());
    assert_eq!(first.len(), MIB);
}

#[test]
fn an_oversized_page_is_released_after_use_and_does_not_flush_other_pages() {
    let pdf = Pdf::new(fixtures::pdf(2, |page| {
        let size = if page == 0 { 128 } else { 33 * MIB };
        Some(vec![vec![b' '; size]])
    }))
    .unwrap();
    let small = pdf.pages()[0].page_stream().unwrap();
    let large = pdf.pages()[1].page_stream().unwrap();
    let weak = Arc::downgrade(&large);
    assert_eq!(large.len(), 33 * MIB);
    assert!(Arc::ptr_eq(&small, &pdf.pages()[0].page_stream().unwrap()));
    drop(large);
    assert!(weak.upgrade().is_none());
}

#[test]
fn content_arrays_keep_separators_and_empty_pages_keep_their_behavior() {
    let pdf = Pdf::new(fixtures::pdf(3, |page| match page {
        0 => Some(vec![
            b"BT /F1 12 Tf 20 740 Td".to_vec(),
            b"(Split text) Tj ET".to_vec(),
        ]),
        1 => Some(Vec::new()),
        _ => None,
    }))
    .unwrap();
    let interpreter = InterpreterCache::new();
    assert_eq!(
        crate::text::extract(&pdf.pages()[0], &interpreter).text,
        "Split text"
    );
    assert_eq!(
        pdf.pages()[0].page_stream().unwrap().as_slice(),
        b"BT /F1 12 Tf 20 740 Td (Split text) Tj ET "
    );
    assert!(pdf.pages()[1].page_stream().unwrap().is_empty());
    assert!(pdf.pages()[2].page_stream().is_none());
    for page in &pdf.pages()[1..] {
        assert!(crate::text::extract(page, &interpreter).text.is_empty());
    }
}

#[test]
fn different_documents_never_share_page_number_cache_entries() {
    let pdf = |text: &[u8]| Pdf::new(fixtures::pdf(1, |_| Some(vec![text.to_vec()]))).unwrap();
    let first = pdf(b"BT /F1 12 Tf 20 740 Td (First book) Tj ET");
    let second = pdf(b"BT /F1 12 Tf 20 740 Td (Second book) Tj ET");
    let interpreter = InterpreterCache::new();
    assert_eq!(
        crate::text::extract(&first.pages()[0], &interpreter).text,
        "First book"
    );
    assert_eq!(
        crate::text::extract(&second.pages()[0], &interpreter).text,
        "Second book"
    );
    assert!(!Arc::ptr_eq(
        &first.pages()[0].page_stream().unwrap(),
        &second.pages()[0].page_stream().unwrap(),
    ));
}

#[test]
fn inline_image_data_is_consumed_before_releasing_the_snapshot() {
    let mut content = b"q 50 0 0 50 10 10 cm BI /W 1 /H 1 /CS /RGB /BPC 8 ID ".to_vec();
    content.extend([255, 0, 0]);
    content.extend(b" EI Q\nBT /F1 12 Tf 20 740 Td (Inline image) Tj ET");
    let pdf = Pdf::new(fixtures::pdf(1, |_| Some(vec![content.clone()]))).unwrap();
    let cache = RenderCache::new();
    let bitmap = rasterize_untransformed(&pdf, &cache, 0, 600).unwrap();
    assert!(bitmap.rgba.as_chunks::<4>().0.contains(&[255, 0, 0, 255]));
    let text = crate::text::extract(&pdf.pages()[0], &InterpreterCache::new());
    assert_eq!(text.text, "Inline image");
}

#[test]
fn concurrent_readers_share_the_admitted_snapshot() {
    let pdf = Arc::new(Pdf::new(fixtures::compressed(1, MIB)).unwrap());
    let barrier = Arc::new(std::sync::Barrier::new(4));
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let pdf = Arc::clone(&pdf);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                pdf.pages()[0].page_stream().unwrap()
            })
        })
        .collect();
    let streams: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    assert!(
        streams
            .iter()
            .all(|stream| Arc::ptr_eq(stream, &streams[0]))
    );
}
