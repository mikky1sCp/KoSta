// kosta-server/src/web/handlers/tests.rs
use super::*;

#[test]
fn percent_encoding_keeps_ascii_and_encodes_unicode() {
    assert_eq!(
        percent_encode_filename("photo-1_final.png"),
        "photo-1_final.png"
    );
    assert_eq!(
        percent_encode_filename("фото 1.png"),
        "%D1%84%D0%BE%D1%82%D0%BE%201.png"
    );
    assert_eq!(percent_encode_filename("a+b;c.pdf"), "a%2Bb%3Bc.pdf");
}
