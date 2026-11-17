#![no_main]

use libfuzzer_sys::fuzz_target;
use soulseek_rs::message::Message;
use soulseek_rs::utils::zlib::{deflate, inflate_limited};

const MAX_INFLATED_BYTES: usize = 1024 * 1024;

fuzz_target!(|data: &[u8]| {
    if let Ok(inflated) = inflate_limited(data, MAX_INFLATED_BYTES) {
        assert!(inflated.len() <= MAX_INFLATED_BYTES);

        let mut message = Message::new_with_data(inflated);
        let _ = message.read_string();
        let _ = message.read_int32();
        let _ = message.read_int64();
    }

    let compressed = deflate(data);
    let inflated = inflate_limited(&compressed, MAX_INFLATED_BYTES)
        .expect("our valid compressed input must inflate within the bound");
    assert_eq!(inflated, data);
});
