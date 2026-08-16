use gt_loader::derive_identity;

#[test]
fn test_no_recursive_identity() {
    let id1 = derive_identity(None, None, None, "file.gtd");
    assert_eq!(id1, "auto:file.gtd");

    // Simulate recursive loading: derived identity used as filename
    let id2 = derive_identity(None, None, None, &id1);

    assert_eq!(id2, "auto:file.gtd");
}
