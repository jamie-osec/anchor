use anchor_lang_v2::{wincode, BORSH_CONFIG};

#[test]
fn borsh_config_still_deserializes_align_one_borrowed_bytes() {
    let values: &[u8] = b"anchor borrow checks";
    let encoded = wincode::config::serialize(values, BORSH_CONFIG).unwrap();

    let decoded: &[u8] = wincode::config::deserialize(&encoded, BORSH_CONFIG).unwrap();
    assert_eq!(decoded, values);
}
