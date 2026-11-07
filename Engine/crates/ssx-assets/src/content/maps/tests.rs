use super::*;

#[test]
fn a_layout_sits_on_a_disc_drop_and_a_map_lights_with_a_disc_preset() {
    let dir = std::env::temp_dir().join(format!("content11-maps-{}", std::process::id()));
    let content = GameContent::of(&dir);
    let drop: ContentId = "pack:drop/x".parse().expect("an id");
    assert!(content.layout_base(&drop).is_err());
    let lighting: ContentId = "pack:lighting/x".parse().expect("an id");
    assert!(content.map_lighting(&lighting).is_err());
    let not_a_map: ContentId = "pack:object/x".parse().expect("an id");
    assert!(content.map(&not_a_map).is_err());
    assert!(matches!(content.map_object(&"pack:object/x".parse().expect("an id")), Err(LoadError::Missing(_))));
}
