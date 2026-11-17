#![no_main]

use libfuzzer_sys::fuzz_target;
use soulseek_rs::message::peer::{
    SharedDirectory, SharedFileEntry, build_folder_contents, build_shared_file_list,
    parse_folder_contents_limited, parse_shared_file_list_limited,
};
use soulseek_rs::message::{Message, distributed};

const MAX_LISTING_BYTES: usize = 1024 * 1024;

fuzz_target!(|data: &[u8]| {
    let mut message = Message::new_with_data(data.to_vec());
    let _ = distributed::parse(&mut message);

    let mut message = Message::new_with_data(data.to_vec());
    let _ = message.read_string();
    let _ = message.read_int8();
    let _ = message.read_int32();
    let _ = message.read_int64();
    let _ = message.read_bool();

    let mut message = Message::new_with_data(data.to_vec());
    let _ = parse_shared_file_list_limited(&mut message, MAX_LISTING_BYTES);

    let mut message = Message::new_with_data(data.to_vec());
    let _ = parse_folder_contents_limited(&mut message, MAX_LISTING_BYTES);

    let name = String::from_utf8_lossy(data).into_owned();
    let directories = vec![SharedDirectory {
        name: name.clone(),
        files: vec![SharedFileEntry {
            name: name.clone(),
            size: data.len() as u64,
            attributes: vec![(0, data.len() as u32)],
        }],
    }];

    let built = build_shared_file_list(&directories);
    let mut listing = Message::new_with_data(built.get_buffer());
    listing.set_pointer(8);
    assert_eq!(
        parse_shared_file_list_limited(&mut listing, MAX_LISTING_BYTES),
        directories
    );

    let built = build_folder_contents(1, &name, &directories);
    let mut folder = Message::new_with_data(built.get_data());
    folder.set_pointer(4);
    assert_eq!(
        parse_folder_contents_limited(&mut folder, MAX_LISTING_BYTES),
        Some((1, name, directories))
    );
});
