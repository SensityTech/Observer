//! Jenkins' one-at-a-time hash, which RAGE calls `joaat`. Names the game looks up (archetypes, assets,
//! files) are hashed lower-cased with `\` read as `/`, as `atStringHash` does; the names of metadata
//! structures and fields are hashed as written.

/// The hash of `bytes` as they are.
pub const fn joaat(bytes: &[u8]) -> u32 {
    let mut hash = 0u32;
    let mut index = 0;
    while index < bytes.len() {
        hash = mix(hash, bytes[index]);
        index += 1;
    }
    finish(hash)
}

/// The hash the game gives a name: ASCII letters lower-cased and `\` read as `/`.
pub fn name_hash(name: &str) -> u32 {
    let hash = name.bytes().fold(0u32, |hash, byte| {
        let byte = match byte {
            b'\\' => b'/',
            byte => byte.to_ascii_lowercase(),
        };
        mix(hash, byte)
    });
    finish(hash)
}

const fn mix(hash: u32, byte: u8) -> u32 {
    let hash = hash.wrapping_add(byte as u32);
    let hash = hash.wrapping_add(hash << 10);
    hash ^ (hash >> 6)
}

const fn finish(hash: u32) -> u32 {
    let hash = hash.wrapping_add(hash << 3);
    let hash = hash ^ (hash >> 11);
    hash.wrapping_add(hash << 15)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structure_names_hash_as_written() {
        assert_eq!(joaat(b"CMapTypes"), 0xD98B_B561);
        assert_eq!(joaat(b"CBaseArchetypeDef"), 0x82D6_FC83);
        assert_eq!(joaat(b"CTimeArchetypeDef"), 0x76B0_C56C);
        assert_eq!(joaat(b"CMloArchetypeDef"), 0x1050_6455);
    }

    #[test]
    fn archetype_names_hash_as_the_client_logs_them() {
        assert_eq!(name_hash("bzzz_camp_food_marshmallow"), 0xDE47_BB3D);
        assert_eq!(name_hash("bz_prop_gift"), 0x5201_AE80);
        assert_eq!(name_hash("w_at_ar_nutscrackercarbineriflesupp"), 0x7017_567B);
    }

    #[test]
    fn a_name_hashes_the_same_whatever_its_case_and_slashes() {
        assert_eq!(
            name_hash("BZZZ_Camp_Food_Marshmallow"),
            name_hash("bzzz_camp_food_marshmallow")
        );
        assert_eq!(name_hash("stream\\props"), name_hash("stream/props"));
        assert_ne!(joaat(b"CMapTypes"), name_hash("CMapTypes"));
    }
}
