use super::*;

#[test]
fn an_export_names_the_kinds_of_its_filters_never_their_values() {
    let query = LogQuery {
        category: Some(DropCategory::Crash),
        player: Some("license:0123456789abcdef0123456789abcdef01234567".to_owned()),
        ..LogQuery::default()
    };

    assert_eq!(query.kinds(), ["category", "player"]);
}

#[test]
fn a_cursor_is_the_last_rows_end_and_id() {
    assert_eq!(parse_cursor("1761912000000.42").unwrap(), (1_761_912_000_000, 42));
    assert!(parse_cursor("42").is_err());
    assert!(parse_cursor("a.b").is_err());
}
