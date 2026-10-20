use super::*;

fn compiled(extra: &str) -> CompiledProfile {
    CompiledProfile::compile(Profile::from_toml(&format!("{}\n{extra}", crate::DEFAULT_PROFILE_TOML)).unwrap()).unwrap()
}

#[test]
fn without_plugin_kinds_or_overrides_the_profile_order_is_kept() {
    let mut c = compiled("");
    let before: Vec<String> = c.markers.iter().map(|m| m.kind.clone()).collect();
    assert!(apply(&mut c, &[]).unwrap().is_empty());
    assert_eq!(before, c.markers.iter().map(|m| m.kind.clone()).collect::<Vec<_>>());
    assert!(c.markers.iter().all(|m| m.owner == OWNER_PROFILE), "classified only when merging");
}

#[test]
fn builtins_are_recognised_and_ordered_last() {
    let mut c = compiled("[[marker]]\nkind = 'mine'\nwhen = [{ field = '@message', op = 'contains', value = 'x' }]");
    let k = PluginMarkerKind::new("k", "K")
        .when_in("EventId", &["1", "2"])
        .role(MarkerRole::Start);
    apply(&mut c, &[("p".into(), k)]).unwrap();
    let owners: Vec<&str> = c.markers.iter().map(|m| m.owner.as_str()).collect();
    assert_eq!(&owners[..3], ["profile", "p", "builtin"]);
    assert_eq!(c.markers[1].kind, "p/k");
    assert!(c.lifecycle_mask & 0b10 != 0);
    let start = c.markers.iter().position(|m| m.kind == "start").unwrap();
    assert!(c.lifecycle_mask & (1 << start) != 0);
}

#[test]
fn mark_index_and_scope() {
    let mut c = compiled("");
    apply(
        &mut c,
        &[
            ("p".into(), PluginMarkerKind::new("a", "A")),
            ("q".into(), PluginMarkerKind::new("b", "B")),
        ],
    )
    .unwrap();
    let index = MarkIndex::of(&c.markers);
    let scope = MarkScope::new("p");
    assert!(scope.mark(&index, "a"));
    assert!(scope.mark(&index, "p/a"));
    assert!(!scope.mark(&index, "q/b"), "another plugin's kind");
    assert!(!scope.mark(&index, "error"));
    assert!(!scope.reject("x"));
    let (mask, rejected) = scope.finish();
    assert_eq!(mask, 1);
    assert_eq!(rejected, ["q/b", "error", "x"]);
    // Contexts without a scope or index refuse marks.
    assert!(!crate::plugin::TransformCtx::new("f").mark("a"));
    let scope = MarkScope::new("p");
    let ctx = crate::plugin::TransformCtx::new("f");
    assert!(!ctx.scoped(&scope).mark("a"));
    assert_eq!(scope.finish().1, ["a"]);
}

#[test]
fn visibility_hides_unavailable_owners_only() {
    let mut c = compiled("");
    apply(&mut c, &[("p".into(), PluginMarkerKind::new("a", "A"))]).unwrap();
    let all = visible_mask(&c.markers, |_| true);
    let hidden = visible_mask(&c.markers, |o| o != "p");
    assert_eq!(all.count_ones() as usize, c.markers.len());
    assert_eq!(all & !hidden, 1);
}

#[test]
fn builder_sets_every_field() {
    let k = PluginMarkerKind::new("k", "K")
        .describe("d")
        .color("#123")
        .icon("flag")
        .badge("AB", "square")
        .role(MarkerRole::Stop)
        .when("@message", "contains", "x");
    assert_eq!(
        (
            k.description.as_deref(),
            k.color.as_deref(),
            k.icon.as_deref(),
            k.badge.as_deref(),
            k.shape.as_deref()
        ),
        (Some("d"), Some("#123"), Some("flag"), Some("AB"), Some("square"))
    );
    super::check(&k, "p").unwrap();
    assert!(super::check(&PluginMarkerKind::new("", "x"), "p").is_err());
    assert!(super::check(&PluginMarkerKind::new(&"x".repeat(65), "x"), "p").is_err());
    assert!(super::check(&PluginMarkerKind::new("k", "x").color("blue"), "p").is_err());
    assert_eq!(serde_json::to_string(&MarkerRole::Crash).unwrap(), "\"crash\"");
}
