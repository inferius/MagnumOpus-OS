use super::*;

#[test]
fn test_code_in_release_initfs_is_error() {
    let clean: &[u8] = b"init: vsechny sluzby bezi";
    assert!(check_release("test", [("init", clean)]).is_ok());
    let with_tests: &[u8] = b"xx 459 testu userspace proslo, 0 selhalo yy";
    let e = check_release("test", [("echo", clean), ("init", with_tests)]).unwrap_err().to_string();
    assert!(e.contains("init") && e.contains("kod testu"), "{e}");
}

#[test]
fn markers_are_distinct_from_dev_mode() {
    for m in MARKERS {
        assert!(!crate::devmode::MARKERS.contains(m), "{m}");
    }
}
