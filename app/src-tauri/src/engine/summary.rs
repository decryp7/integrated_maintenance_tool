//! The `WorkspaceSummary` the UI shows: what the store knows (sources, totals, fields, the instance key, marker
//! kinds by bit) with what the window's host knows (the profile's charts, icons and dashboards, marker kind
//! details and plugin rules, plugins, export formats) and the last import's time.

use super::Host;
use imt_core::api::{MarkerKind, WorkspaceSummary};
use imt_core::ports::{ImportReport, SummaryReply};

/// The summary of a store's `reply` for the window's `host`.
pub(crate) fn compose(reply: SummaryReply, host: &Host, last: Option<&ImportReport>) -> WorkspaceSummary {
    let compiled = host.compiled();
    let registry = &host.registry;
    let visible = registry.visible_markers(&compiled.markers);
    // The store numbers marker kinds as the profile compiled with the same plugins does; a kind the host does
    // not know (another profile in the store) is listed by name only.
    let marker_kinds = reply
        .marker_kinds
        .iter()
        .filter_map(|name| {
            let (i, m) = compiled.markers.iter().enumerate().find(|(_, m)| &m.kind == name)?;
            (visible & (1 << i) != 0).then(|| MarkerKind {
                kind: m.kind.clone(),
                owner: m.owner.clone(),
                role: m.role,
                label: m.label.clone(),
                lifecycle: m.lifecycle,
                color: m.color.clone(),
                icon: m.icon.clone(),
                badge: m.badge.clone(),
                shape: m.shape.clone(),
                description: m.description.clone(),
                when: registry.marker_rule(&m.owner, &m.kind),
            })
        })
        .collect();
    WorkspaceSummary {
        dropped: reply.sources.iter().map(|s| s.dropped).sum(),
        sources: reply.sources,
        unrecognized: reply.unrecognized,
        total: reply.total,
        untimed: reply.untimed,
        from: reply.from,
        to: reply.to,
        levels: reply.levels,
        fields: reply.fields,
        default_group_by: reply.default_group_by,
        marker_kinds,
        charts: imt_core::chart::chart_defs(&compiled.profile),
        profile_name: reply.profile_name,
        load_ms: last.map_or(0, |r| r.elapsed_ms),
        icons: compiled.icons.clone(),
        pipeline: Vec::new(),
        plugins: registry.plugins(),
        exporters: registry.exporter_infos(),
        dashboards: compiled.profile.dashboard.clone(),
        redacted_view: host.redacted_view().is_some(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_core::{PluginRegistry, Profile};

    #[test]
    fn the_summary_joins_the_store_and_the_host() {
        let profile = Profile::from_toml(
            r#"
name = "T"
[[marker]]
kind = "boom"
when = [{ field = "@message", op = "contains", value = "boom" }]
"#,
        )
        .unwrap();
        let host = Host::new(PluginRegistry::new(), profile).unwrap();
        let reply = SummaryReply {
            total: 3,
            untimed: 1,
            from: Some(1),
            to: Some(2),
            levels: Default::default(),
            sources: vec![],
            unrecognized: vec![],
            fields: vec![],
            default_group_by: vec!["Host".into()],
            marker_kinds: vec!["boom".into(), "unknown".into()],
            profile_name: "T".into(),
        };
        let last = ImportReport {
            elapsed_ms: 42,
            ..Default::default()
        };
        let s = compose(reply, &host, Some(&last));
        assert_eq!((s.total, s.untimed, s.load_ms), (3, 1, 42));
        assert_eq!(s.marker_kinds.iter().map(|m| m.kind.as_str()).collect::<Vec<_>>(), ["boom"]);
        assert!(s.exporters.iter().any(|e| e.id == "csv"));
        assert_eq!(s.default_group_by, ["Host"]);
    }
}
