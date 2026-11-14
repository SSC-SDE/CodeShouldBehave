use super::*;
#[test]
fn defaults_and_strict_validation() {
    let mut doc = defaults();
    assert!(validate(&doc).is_ok());
    doc["modes"]["focus"]["visibility"]["architecture"] = json!(false);
    assert!(validate(&doc).is_err());
    doc = defaults();
    doc["credential"] = json!("not accepted");
    assert!(validate(&doc).is_err());
    doc = defaults();
    doc["modes"]["standard"]["widths"]["left"] = json!(421);
    assert!(validate(&doc).is_err());
}
#[test]
fn theme_validation_and_legacy_documents() {
    let mut doc = defaults();
    for theme in ["graphite", "blueprint", "terminal", "contrast"] {
        doc["theme"] = json!(theme);
        assert!(validate(&doc).is_ok());
    }
    doc["theme"] = json!("custom-css");
    assert!(validate(&doc).is_err());
    doc.as_object_mut().unwrap().remove("theme");
    assert!(validate(&doc).is_ok());
}
#[test]
fn hld_appearance_and_legacy_modes() {
    let mut doc = defaults();
    assert_eq!(doc["modes"]["focus"]["hldAppearance"], "studio");
    assert_eq!(doc["modes"]["standard"]["hldAppearance"], "classic");
    doc["modes"]["focus"]
        .as_object_mut()
        .unwrap()
        .remove("hldAppearance");
    assert!(validate(&doc).is_ok());
    doc["modes"]["focus"]["hldAppearance"] = json!("classic");
    assert!(validate(&doc).is_ok());
    doc["modes"]["focus"]["hldAppearance"] = json!("custom-css");
    assert!(validate(&doc).is_err());
}
#[test]
fn account_isolation_and_atomic_revision_conflicts() {
    let root = std::env::temp_dir().join(format!("csb-preferences-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let mut storage = Storage::open(&root.join("state.sqlite"), &root.join("artifacts")).unwrap();
    let a = PrincipalId::new("github-user:11").unwrap();
    let b = PrincipalId::new("github-user:12").unwrap();
    let first = read(&storage, &a).unwrap();
    let revision = first["revision"].as_i64().unwrap();
    let mut doc = defaults();
    doc["defaultMode"] = json!("focus");
    let request = json!({"revision":revision,"preferences":doc});
    assert_eq!(
        write(&mut storage, &a, &request).unwrap()["revision"],
        revision + 1
    );
    assert_eq!(
        write(&mut storage, &a, &request).unwrap_err(),
        "Preference conflict"
    );
    assert_eq!(read(&storage, &b).unwrap()["revision"], 0);
    drop(storage);
    let storage = Storage::open(&root.join("state.sqlite"), &root.join("artifacts")).unwrap();
    assert_eq!(
        read(&storage, &a).unwrap()["preferences"]["defaultMode"],
        "focus"
    );
    drop(storage);
    std::fs::remove_dir_all(root).unwrap();
}

fn legacy_preferences() -> Value {
    let mut doc = defaults();
    doc["theme"] = json!("terminal");
    doc["modes"]["focus"]["widths"]["right"] = json!(380);
    for name in ["standard", "focus"] {
        doc["modes"][name]["visibility"]
            .as_object_mut()
            .unwrap()
            .remove("focusBackdrop");
        doc["modes"][name]["visibility"]
            .as_object_mut()
            .unwrap()
            .remove("ringBackdrop");
        doc["modes"][name]["order"]["main"]
            .as_array_mut()
            .unwrap()
            .retain(|id| id != "focusBackdrop" && id != "ringBackdrop");
        doc["modes"][name]["order"]["main"]
            .as_array_mut()
            .unwrap()
            .reverse();
    }
    doc
}
#[test]
fn legacy_backdrop_migration_preserves_preferences_and_order() {
    let legacy = legacy_preferences();
    assert!(validate(&legacy).is_ok());
    let mut doc = legacy.clone();
    migrate(&mut doc);
    assert_eq!(doc["theme"], "terminal");
    assert_eq!(doc["modes"]["focus"]["widths"]["right"], 380);
    for name in ["standard", "focus"] {
        assert_eq!(
            doc["modes"][name]["visibility"]["focusBackdrop"],
            name == "focus"
        );
        assert_eq!(
            doc["modes"][name]["visibility"]["ringBackdrop"],
            name == "standard"
        );
        let order = doc["modes"][name]["order"]["main"].as_array().unwrap();
        assert_eq!(
            &order[..order.len() - 2],
            legacy["modes"][name]["order"]["main"].as_array().unwrap()
        );
    }
    let once = doc.clone();
    migrate(&mut doc);
    assert_eq!(doc, once);
    doc["modes"]["focus"]["visibility"]["focusBackdrop"] = json!(false);
    migrate(&mut doc);
    assert_eq!(doc["modes"]["focus"]["visibility"]["focusBackdrop"], false);
    doc["modes"]["standard"]["visibility"]["ringBackdrop"] = json!(false);
    doc["modes"]["focus"]["visibility"]["ringBackdrop"] = json!(true);
    migrate(&mut doc);
    assert_eq!(
        doc["modes"]["standard"]["visibility"]["ringBackdrop"],
        false
    );
    assert_eq!(doc["modes"]["focus"]["visibility"]["ringBackdrop"], true);
    assert!(validate(&doc).is_ok());
    doc["modes"]["standard"]["visibility"]["ringBackdrop"] = json!("yes");
    assert!(validate(&doc).is_err());
    doc["modes"]["standard"]["visibility"]["ringBackdrop"] = json!(false);
    doc["modes"]["standard"]["visibility"]
        .as_object_mut()
        .unwrap()
        .remove("files");
    assert!(validate(&doc).is_err());
}
#[test]
fn legacy_backdrop_documents_read_and_save_with_account_revision() {
    let root =
        std::env::temp_dir().join(format!("csb-backdrop-preferences-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let mut storage = Storage::open(&root.join("state.sqlite"), &root.join("artifacts")).unwrap();
    let account = PrincipalId::new("github-user:13").unwrap();
    let legacy = legacy_preferences();
    storage
        .connection
        .execute(
            "INSERT INTO workspace_preferences(principal,revision,document) VALUES (?1,4,?2)",
            params![account.as_str(), legacy.to_string()],
        )
        .unwrap();
    let loaded = read(&storage, &account).unwrap();
    assert_eq!(loaded["revision"], 4);
    assert_eq!(
        loaded["preferences"]["modes"]["focus"]["visibility"]["focusBackdrop"],
        true
    );
    assert_eq!(
        loaded["preferences"]["modes"]["standard"]["visibility"]["ringBackdrop"],
        true
    );
    let saved = write(
        &mut storage,
        &account,
        &json!({"revision":4,"preferences":legacy}),
    )
    .unwrap();
    assert_eq!(saved["revision"], 5);
    assert_eq!(saved["preferences"]["theme"], "terminal");
    assert_eq!(
        saved["preferences"]["modes"]["standard"]["visibility"]["focusBackdrop"],
        false
    );
    assert_eq!(
        saved["preferences"]["modes"]["focus"]["visibility"]["ringBackdrop"],
        false
    );
    drop(storage);
    std::fs::remove_dir_all(root).unwrap();
}
