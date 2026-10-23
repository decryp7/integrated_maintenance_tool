use super::*;
use crate::entities::{extract, Entity};

#[test]
fn scopes_round_trip() {
    for s in [
        Scope::Entity(EntityKind::Host),
        Scope::Field("user".into()),
        Scope::Field("a:b".into()),
    ] {
        assert_eq!(Scope::decode(&s.encode()), Some(s));
    }
    assert_eq!(Scope::decode("bogus"), None);
}

#[test]
fn trigram_keys_of_names_and_searches() {
    assert_eq!(trigram_keys("abcd"), ["abc", "bcd"]);
    assert!(trigram_keys("ab").is_empty());
    assert_eq!(trigram_keys("aaaa"), ["aaa"], "distinct");
    let k = search_keys("grafic bilder");
    assert!(
        k.contains(&"gra".to_string()) && k.contains(&"bil".to_string()) && k.contains(&"icb".to_string()),
        "{k:?}"
    );
    assert_eq!(squash("Graphic.Builder_1"), "graphicbuilder1");
}

#[test]
fn a_tally_counts_and_orders_deterministically() {
    let mut t = Tally::default();
    assert!(t.is_empty());
    t.add(Scope::Entity(EntityKind::Host), "Web.example.com", 5, Some(50), 1);
    t.add(Scope::Entity(EntityKind::Host), "web.example.com", 2, None, 1);
    t.add(Scope::Entity(EntityKind::Host), "web.example.com", 9, Some(10), 1);
    t.add(Scope::Field("user".into()), "bob", 1, Some(1), 7);
    t.add(Scope::Field("user".into()), "", 1, None, 1);
    t.add(Scope::Field("user".into()), &"x".repeat(MAX_NAME_CHARS + 1), 1, None, 1);
    assert_eq!(t.len(), 2);
    let v = t.finish();
    assert_eq!((v[0].name.as_str(), v[0].count), ("bob", 7));
    let h = &v[1];
    assert_eq!(
        (h.name.as_str(), h.count, h.first_ts, h.last_ts, h.first_id, h.last_id),
        ("Web.example.com", 3, Some(10), Some(50), 2, 9)
    );
}

#[test]
fn a_full_tally_drops_new_names_but_counts_known_ones() {
    let mut t = Tally::default();
    for i in 0..MAX_NAMES + 10 {
        t.add(Scope::Field("f".into()), &format!("v{i}"), i as u64, None, 1);
    }
    assert_eq!((t.len(), t.dropped), (MAX_NAMES, 10));
    t.add(Scope::Field("f".into()), "v0", 1, None, 1);
    assert_eq!(t.dropped, 10);
    assert_eq!(t.finish()[0].count, 2);
}

#[test]
fn guids_are_all_counted_and_a_few_listed() {
    let mut t = Tally::default();
    for i in 0..MAX_GUIDS + 50 {
        let g = format!("{i:08x}-0000-0000-0000-000000000000");
        let found = extract(&[&format!("id {g}")]);
        assert_eq!(found.guids, 1);
        t.add_event(&found, i as u64, None);
    }
    t.add_event(
        &Extracted {
            entities: vec![Entity {
                kind: EntityKind::Guid,
                name: "00000000-0000-0000-0000-000000000000".into(),
            }],
            guids: 1,
        },
        1,
        None,
    );
    assert_eq!(t.guids, (MAX_GUIDS + 51) as u64);
    assert_eq!(t.finish().len(), MAX_GUIDS);
}

#[test]
fn candidates_are_found_by_shared_groups_and_merged_across_sources() {
    let entry = |name: &str, count: u64| Entry {
        scope: Scope::Entity(EntityKind::Executable),
        name: name.into(),
        count,
        first_ts: Some(count as i64),
        last_ts: Some(count as i64 * 2),
        first_id: 1,
        last_id: 2,
    };
    let rows = vec![
        ("b.log".to_string(), entry("CompanyA.Graphic.Builder.exe", 2)),
        ("a.log".to_string(), entry("CompanyA.Graphic.Builder.exe", 3)),
        ("a.log".to_string(), entry("other.exe", 9)),
    ];
    let c = candidates_from(&rows, "grafic bilder");
    assert_eq!(c.len(), 1, "{c:?}");
    assert_eq!((c[0].count, c[0].first_ts, c[0].last_ts), (5, Some(2), Some(6)));
    assert_eq!(c[0].seen_in, ["a.log", "b.log"]);
    assert_eq!(candidates_from(&rows, "ot")[0].name, "other.exe", "short text: a prefix");
    assert!(candidates_from(&rows, "--").is_empty() && candidates_from(&rows, "zzzz").is_empty());
}
