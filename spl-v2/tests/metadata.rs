#![cfg(feature = "metadata")]

use {
    anchor_lang_v2::{testing::AccountBuffer, AccountDeserialize, AnchorAccount},
    anchor_spl_v2::metadata::{self, MetadataAccount},
    borsh::to_vec,
    solana_program_error::ProgramError,
    solana_pubkey::Pubkey,
};

fn sample_metadata() -> mpl_token_metadata::accounts::Metadata {
    let creator = Pubkey::from([7u8; 32]);
    mpl_token_metadata::accounts::Metadata {
        key: mpl_token_metadata::types::Key::MetadataV1,
        update_authority: Pubkey::from([1u8; 32]),
        mint: Pubkey::from([2u8; 32]),
        name: "Pump AMM".to_string(),
        symbol: "PUMP".to_string(),
        uri: "https://example.invalid/pump.json".to_string(),
        seller_fee_basis_points: 250,
        creators: Some(vec![mpl_token_metadata::types::Creator {
            address: creator,
            verified: true,
            share: 100,
        }]),
        primary_sale_happened: false,
        is_mutable: true,
        edition_nonce: Some(255),
        token_standard: None,
        collection: None,
        uses: None,
        collection_details: None,
        programmable_config: None,
    }
}

#[test]
fn fixture_is_real_metadata_program_elf() {
    let fixture = include_bytes!("fixtures/metaplex_token_metadata.so");
    assert_eq!(fixture.len(), 283_512);
    assert_eq!(&fixture[..4], b"\x7fELF");
}

#[test]
fn metadata_account_deserializes_raw_metaplex_bytes() {
    let expected = sample_metadata();
    let data = to_vec(&expected).unwrap();
    let account = MetadataAccount::try_deserialize(&mut data.as_slice()).unwrap();

    assert_eq!(account.key, mpl_token_metadata::types::Key::MetadataV1);
    assert_eq!(account.name, "Pump AMM");
    assert_eq!(account.mint, expected.mint);
    assert_eq!(
        account.creators.as_ref().unwrap()[0].address,
        Pubkey::from([7u8; 32])
    );
}

#[test]
fn metadata_account_load_validates_owner_and_raw_data() {
    let expected = sample_metadata();
    let data = to_vec(&expected).unwrap();
    let account = AccountBuffer::<4096>::new();
    account.init(
        [9u8; 32],
        metadata::ID.to_bytes(),
        data.len(),
        false,
        false,
        false,
    );
    account.write_data(&data);

    let loaded = MetadataAccount::load(unsafe { account.view() }).unwrap();
    assert_eq!(loaded.update_authority, expected.update_authority);
    assert_eq!(loaded.seller_fee_basis_points, 250);
}

#[test]
fn metadata_account_rejects_wrong_owner() {
    let data = to_vec(&sample_metadata()).unwrap();
    let account = AccountBuffer::<4096>::new();
    account.init([9u8; 32], [3u8; 32], data.len(), false, false, false);
    account.write_data(&data);

    let err = MetadataAccount::load(unsafe { account.view() }).unwrap_err();
    assert_eq!(err, ProgramError::IllegalOwner);
}

#[test]
fn metadata_account_rejects_non_metadata_key_without_anchor_discriminator() {
    let mut data = to_vec(&sample_metadata()).unwrap();
    data[0] = mpl_token_metadata::types::Key::MasterEditionV2 as u8;

    let err = MetadataAccount::try_deserialize(&mut data.as_slice()).unwrap_err();
    assert_eq!(err, ProgramError::InvalidAccountData);
}
