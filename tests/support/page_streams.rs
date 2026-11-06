//! Small generated PDFs with independently compressed page content streams.
use lopdf::{Document, Object, Stream, dictionary};

pub fn pdf(pages: usize, mut contents: impl FnMut(usize) -> Option<Vec<Vec<u8>>>) -> Vec<u8> {
    let mut document = Document::with_version("1.5");
    let parent = document.new_object_id();
    let font = document.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let resources = document.add_object(dictionary! { "Font" => dictionary! { "F1" => font } });
    let mut kids = Vec::new();
    for page in 0..pages {
        let mut entry = dictionary! {
            "Type" => "Page", "Parent" => parent,
            "MediaBox" => vec![0.into(), 0.into(), 600.into(), 800.into()],
            "Resources" => resources,
        };
        if let Some(streams) = contents(page) {
            let ids: Vec<Object> = streams
                .into_iter()
                .map(|content| {
                    let mut stream = Stream::new(dictionary! {}, content);
                    stream.compress().unwrap();
                    document.add_object(stream).into()
                })
                .collect();
            let content = if ids.len() == 1 {
                ids.into_iter().next().unwrap()
            } else {
                Object::Array(ids)
            };
            entry.set("Contents", content);
        }
        kids.push(document.add_object(entry).into());
    }
    document.objects.insert(
        parent,
        dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => pages as i64 }.into(),
    );
    let catalog = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => parent });
    document.trailer.set("Root", catalog);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

pub fn compressed(pages: usize, decoded_bytes: usize) -> Vec<u8> {
    pdf(pages, |page| {
        let body = format!(
            "q 0.1 0.4 0.8 rg 20 20 80 40 re f Q\nBT /F1 12 Tf 20 740 Td (Page {page}) Tj ET"
        );
        assert!(decoded_bytes >= body.len() + 2);
        let mut content = vec![b'x'; decoded_bytes];
        content[0] = b'%';
        let start = decoded_bytes - body.len();
        content[start - 1] = b'\n';
        content[start..].copy_from_slice(body.as_bytes());
        Some(vec![content])
    })
}
