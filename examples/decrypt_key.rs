use nostr_engine::identity::decrypt_ncryptsec;

fn main() {
    let ncryptsec = "<NCRYPTSEC>";
    let password = "<PASSWORD>";
    match decrypt_ncryptsec(ncryptsec, password) {
        Ok((secret_hex, pubkey_hex)) => {
            println!("SECRET_HEX={}", secret_hex);
            println!("PUBKEY_HEX={}", pubkey_hex);
        }
        Err(e) => eprintln!("Error: {:?}", e),
    }
}
