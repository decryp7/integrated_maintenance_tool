//! What a plugin is: its identity, the rights it needs and the contributions it registers.

use crate::annotate::Annotator;
use crate::config::ConfigParser;
use crate::importer::{BinaryImporter, TextImporter};
use crate::manifest::{ConfigParserCap, ImporterCap, Manifest, PermissionRequest, ProfileImporterCap, PROTOCOL};
use crate::markers::PluginMarkerKind;
use crate::model::Record;
use crate::settings::Configurable;
use crate::tools::{Tool, ToolInfo};
use anyhow::Result;
use std::cell::RefCell;
use std::sync::Arc;

/// A plugin. Its process serves it with [`crate::serve`].
pub trait Plugin: Send + Sync + 'static {
    /// Stable, unique id, reverse-DNS style (`net.decryptology.registry`).
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn version(&self) -> &str;
    /// Adds the plugin's contributions. Must not panic; return an error instead (the plugin is then listed
    /// as inactive with it).
    fn register(&self, c: &mut Contributions) -> Result<()>;
    /// Rights beyond parsing what the host hands it.
    fn permissions(&self) -> Vec<PermissionRequest> {
        Vec::new()
    }
    /// Memory it needs at most, in bytes (`None`: the host's default).
    fn memory_limit(&self) -> Option<u64> {
        None
    }
}

/// A profile (or heap snapshot) format. The capture crosses as bytes its library encodes and the host decodes
/// with the same library, so this contract does not depend on the profile model.
pub trait ProfileImporter: Send + Sync {
    fn id(&self) -> &str;
    /// Produces heap snapshots (else profiles).
    fn heap(&self) -> bool;
    /// Confidence 0–100 from the file's name and first bytes.
    fn sniff(&self, file_name: &str, head: &[u8]) -> u8;
    /// Parses the file; returns the encoded result (capture or typed error) for the host.
    fn import_encoded(&self, file_name: &str, bytes: &[u8], limits: &serde_json::Value) -> Vec<u8>;
}

/// What a transform does with a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Keep,
    Drop,
}

/// Context of one transform call.
pub struct TransformCtx<'a> {
    /// Display name of the file the record came from.
    pub source_name: &'a str,
    marks: RefCell<Vec<String>>,
}

impl<'a> TransformCtx<'a> {
    pub fn new(source_name: &'a str) -> TransformCtx<'a> {
        TransformCtx {
            source_name,
            marks: RefCell::new(Vec::new()),
        }
    }

    /// Marks the record being transformed with one of this plugin's marker kinds (short or full name); the
    /// host refuses kinds that are not the plugin's.
    pub fn mark(&self, kind: &str) {
        self.marks.borrow_mut().push(kind.to_string());
    }

    /// The marks made since the last call (the serve loop collects them per record).
    pub fn take_marks(&self) -> Vec<String> {
        std::mem::take(&mut self.marks.borrow_mut())
    }
}

/// A record-level pipeline step (`{ step = "plugin", id = ... }`): deterministic, no shared mutable state.
pub trait Transform: Send + Sync {
    fn id(&self) -> &str;
    fn apply(&self, record: &mut Record, ctx: &TransformCtx) -> Verdict;
}

/// Everything a plugin contributes.
#[derive(Default)]
pub struct Contributions {
    pub(crate) text: Vec<Arc<dyn TextImporter>>,
    pub(crate) binary: Vec<Arc<dyn BinaryImporter>>,
    pub(crate) config_parsers: Vec<Arc<dyn ConfigParser>>,
    pub(crate) marker_kinds: Vec<PluginMarkerKind>,
    pub(crate) annotators: Vec<Arc<dyn Annotator>>,
    pub(crate) transforms: Vec<Arc<dyn Transform>>,
    pub(crate) tools: Vec<(ToolInfo, Arc<dyn Tool>)>,
    pub(crate) settings: Option<Arc<dyn Configurable>>,
    pub(crate) profile_importers: Vec<Arc<dyn ProfileImporter>>,
}

impl Contributions {
    pub fn add_importer(&mut self, importer: impl TextImporter + 'static) -> &mut Self {
        self.text.push(Arc::new(importer));
        self
    }

    pub fn add_binary_importer(&mut self, importer: impl BinaryImporter + 'static) -> &mut Self {
        self.binary.push(Arc::new(importer));
        self
    }

    pub fn add_config_parser(&mut self, parser: impl ConfigParser + 'static) -> &mut Self {
        self.config_parsers.push(Arc::new(parser));
        self
    }

    pub fn add_marker_kinds(&mut self, kinds: Vec<PluginMarkerKind>) -> &mut Self {
        self.marker_kinds.extend(kinds);
        self
    }

    pub fn add_annotator(&mut self, annotator: impl Annotator + 'static) -> &mut Self {
        self.annotators.push(Arc::new(annotator));
        self
    }

    pub fn add_transform(&mut self, transform: impl Transform + 'static) -> &mut Self {
        self.transforms.push(Arc::new(transform));
        self
    }

    /// Adds a tool; its description is taken now.
    pub fn add_tool(&mut self, tool: Arc<dyn Tool>) -> &mut Self {
        let info = tool.describe();
        self.tools.push((info, tool));
        self
    }

    /// The plugin's settings (one section).
    pub fn set_settings(&mut self, settings: Arc<dyn Configurable>) -> &mut Self {
        self.settings = Some(settings);
        self
    }

    pub fn add_profile_importer(&mut self, importer: impl ProfileImporter + 'static) -> &mut Self {
        self.profile_importers.push(Arc::new(importer));
        self
    }

    /// The manifest of `plugin` with these contributions.
    pub fn manifest(&self, plugin: &dyn Plugin) -> Manifest {
        let importers = self
            .text
            .iter()
            .map(|i| ImporterCap {
                id: i.id().into(),
                binary: false,
                tail: true,
                multiline: i.multiline(),
                identity_fields: i.identity_fields(),
            })
            .chain(self.binary.iter().map(|i| ImporterCap {
                id: i.id().into(),
                binary: true,
                tail: false,
                multiline: false,
                identity_fields: i.identity_fields(),
            }))
            .collect();
        Manifest {
            id: plugin.id().into(),
            name: plugin.name().into(),
            version: plugin.version().into(),
            protocol: PROTOCOL,
            permissions: plugin.permissions(),
            memory_limit: plugin.memory_limit(),
            importers,
            config_parsers: self
                .config_parsers
                .iter()
                .map(|p| ConfigParserCap {
                    id: p.id().into(),
                    name: p.name().into(),
                    extensions: p.extensions(),
                })
                .collect(),
            marker_kinds: self.marker_kinds.clone(),
            annotators: self.annotators.iter().map(|a| a.id().into()).collect(),
            transforms: self.transforms.iter().map(|t| t.id().into()).collect(),
            settings: self.settings.as_ref().map(|s| {
                let mut section = s.settings_section();
                section.id = plugin.id().into();
                section.owner = crate::settings::SectionOwner::Plugin { id: plugin.id().into() };
                if section.title.trim().is_empty() {
                    section.title = plugin.name().into();
                }
                section
            }),
            tools: self.tools.iter().map(|(i, _)| i.clone()).collect(),
            profile_importers: self
                .profile_importers
                .iter()
                .map(|p| ProfileImporterCap {
                    id: p.id().into(),
                    heap: p.heap(),
                })
                .collect(),
        }
    }
}
