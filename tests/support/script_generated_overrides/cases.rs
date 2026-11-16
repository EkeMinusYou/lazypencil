//! Decode diagnostic case labels once; fixture consumers use typed dimensions.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Style {
    Direct,
    Descendant,
    Replacement,
    Children,
    Script,
    EmittedChildren,
    EmittedReplacement,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Owner {
    Keep,
    Scope,
    Binding,
    Carrier,
}
impl Owner {
    fn parse(label: &str) -> Self {
        match label {
            "keep" => Self::Keep,
            "scope" => Self::Scope,
            "binding" => Self::Binding,
            "carrier" => Self::Carrier,
            _ => panic!("Unknown code owner: {label}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Inherited {
    Scope,
    StaleScope,
    Context,
    StaleContext,
    Carrier,
    StaleCarrier,
    CarrierOutput,
    UnderlayScope,
    UnderlayBinding,
}
impl Inherited {
    fn parse(label: &str) -> Self {
        match label {
            "scope" => Self::Scope,
            "stale-scope" => Self::StaleScope,
            "context" => Self::Context,
            "stale-context" => Self::StaleContext,
            "carrier" => Self::Carrier,
            "stale-carrier" => Self::StaleCarrier,
            "carrier-output" => Self::CarrierOutput,
            "underlay-scope" => Self::UnderlayScope,
            "underlay-binding" => Self::UnderlayBinding,
            _ => panic!("Unknown inherited state: {label}"),
        }
    }
    pub fn stale(self) -> bool {
        matches!(
            self,
            Self::StaleScope | Self::StaleContext | Self::StaleCarrier
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Context {
    Combined,
    Library,
    Variable,
    Stale,
    Scope,
    Only,
    Output,
    Input,
    Both,
    StaleOutput,
    StaleInput,
    Nested,
    MetadataOutput,
}
impl Context {
    fn parse(label: &str, carrier: bool) -> Self {
        match (carrier, label) {
            (false, "combined") => Self::Combined,
            (false, "library") => Self::Library,
            (false, "variable") => Self::Variable,
            (false, "stale") => Self::Stale,
            (false, "scope") => Self::Scope,
            (false, "only") => Self::Only,
            (true, "output") => Self::Output,
            (true, "input") => Self::Input,
            (true, "both") => Self::Both,
            (true, "stale-output") => Self::StaleOutput,
            (true, "stale-input") => Self::StaleInput,
            (true, "nested") => Self::Nested,
            (true, "metadata-output") => Self::MetadataOutput,
            _ => panic!("Unknown input context: carrier={carrier}, {label}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Combined {
    ScopeInput,
    BindingInput,
    StaleBindingOutput,
    StaleScopeOutput,
    StaleInputs,
    InheritedBindingInput,
    InheritedScopeInput,
    InheritedNonlexical,
}
impl Combined {
    fn parse(label: &str) -> Self {
        match label {
            "scope-input" => Self::ScopeInput,
            "binding-input" => Self::BindingInput,
            "stale-binding-output" => Self::StaleBindingOutput,
            "stale-scope-output" => Self::StaleScopeOutput,
            "stale-inputs" => Self::StaleInputs,
            "inherited-binding-input" => Self::InheritedBindingInput,
            "inherited-scope-input" => Self::InheritedScopeInput,
            "inherited-nonlexical" => Self::InheritedNonlexical,
            _ => panic!("Unknown combined binding: {label}"),
        }
    }
    pub fn inherited(self) -> bool {
        matches!(
            self,
            Self::InheritedBindingInput | Self::InheritedScopeInput | Self::InheritedNonlexical
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NativeScope {
    Scope,
    OnlyScope,
    Custom,
    Default,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    EmittedCode(Owner),
    InheritedState(Inherited),
    CodeOwner {
        variant: Owner,
        native: bool,
    },
    InputContext {
        variant: Context,
        carrier: bool,
    },
    Combined {
        variant: Combined,
        native_parent: bool,
    },
    NativeInput {
        inherited: bool,
        stale: bool,
        scope: NativeScope,
        library: bool,
    },
    NativeParent {
        scope: NativeScope,
        library: bool,
    },
    Literal,
    Variable,
    ThemedVariable,
    BadVariable,
    DefaultInput,
    Metadata,
    ExplicitContext,
    ExplicitInputContext,
    UnknownContext,
    ExplicitBindings,
    BindingLibrary,
    BindingMapped,
    BindingCatalog,
    BindingInputs,
    BindingStaleInputs,
    BindingInheritedInputs,
    BindingMetadata,
    BindingUnknownTarget,
    BindingUnknownLibrary,
    BindingUnmanifestedAsset,
    BindingCombined,
    StaleScope,
    StaleInputScope,
    ActiveInputScope,
    InheritedInputScope,
    StaleInheritedInputScope,
    InheritedUnknownInputScope,
    MetadataOutput,
    MetadataInput,
    MetadataBoth,
    MetadataUnknownSource,
    Inherit,
    SavedInputOwner,
}
impl Mode {
    fn parse(label: &str) -> Self {
        if let Some(value) = label.strip_prefix("binding-emitted-code-") {
            return Self::EmittedCode(Owner::parse(value));
        }
        if let Some(value) = label.strip_prefix("binding-inherited-state-") {
            return Self::InheritedState(Inherited::parse(value));
        }
        if let Some(value) = label.strip_prefix("binding-code-owner-") {
            let native = value.ends_with("-native");
            return Self::CodeOwner {
                variant: Owner::parse(value.strip_suffix("-native").unwrap_or(value)),
                native,
            };
        }
        for (prefix, carrier) in [
            ("binding-input-context-", false),
            ("binding-carrier-", true),
        ] {
            if let Some(value) = label.strip_prefix(prefix) {
                return Self::InputContext {
                    variant: Context::parse(value, carrier),
                    carrier,
                };
            }
        }
        for (prefix, native_parent) in [
            ("binding-combined-", false),
            ("binding-native-parent-combined-", true),
        ] {
            if let Some(value) = label.strip_prefix(prefix) {
                return Self::Combined {
                    variant: Combined::parse(value),
                    native_parent,
                };
            }
        }
        if let Some(value) = label.strip_prefix("binding-native-parent-input-") {
            let library = value.ends_with("-library");
            let value = value.strip_suffix("-library").unwrap_or(value);
            let inherited = value.starts_with("inherited-");
            let value = value.strip_prefix("inherited-").unwrap_or(value);
            let stale = value.ends_with("-stale");
            let value = value.strip_suffix("-stale").unwrap_or(value);
            let scope = match value {
                "scope" => NativeScope::Scope,
                "only-scope" => NativeScope::OnlyScope,
                "custom" => NativeScope::Custom,
                "default" => NativeScope::Default,
                _ => panic!("Unknown native input scope: {label}"),
            };
            return Self::NativeInput {
                inherited,
                stale,
                scope,
                library,
            };
        }
        if label == "binding-native-parent" || label.starts_with("binding-native-parent-") {
            let value = label.strip_prefix("binding-native-parent").unwrap();
            let library = value.ends_with("-library");
            let value = value.strip_suffix("-library").unwrap_or(value);
            let scope = match value {
                "" => NativeScope::Custom,
                "-scope" => NativeScope::Scope,
                "-default" => NativeScope::Default,
                _ => panic!("Unknown native parent: {label}"),
            };
            return Self::NativeParent { scope, library };
        }
        match label {
            "literal" => Self::Literal,
            "variable" => Self::Variable,
            "themed-variable" => Self::ThemedVariable,
            "bad-variable" => Self::BadVariable,
            "default" => Self::DefaultInput,
            "metadata" => Self::Metadata,
            "explicit-context" => Self::ExplicitContext,
            "explicit-input-context" => Self::ExplicitInputContext,
            "unknown-context" => Self::UnknownContext,
            "explicit-bindings" => Self::ExplicitBindings,
            "binding-library" => Self::BindingLibrary,
            "binding-mapped" => Self::BindingMapped,
            "binding-catalog" => Self::BindingCatalog,
            "binding-inputs" => Self::BindingInputs,
            "binding-stale-inputs" => Self::BindingStaleInputs,
            "binding-inherited-inputs" => Self::BindingInheritedInputs,
            "binding-metadata" => Self::BindingMetadata,
            "binding-unknown-target" => Self::BindingUnknownTarget,
            "binding-unknown-library" => Self::BindingUnknownLibrary,
            "binding-unmanifested-asset" => Self::BindingUnmanifestedAsset,
            "binding-combined" => Self::BindingCombined,
            "stale-scope" => Self::StaleScope,
            "stale-input-scope" => Self::StaleInputScope,
            "active-input-scope" => Self::ActiveInputScope,
            "inherited-input-scope" => Self::InheritedInputScope,
            "stale-inherited-input-scope" => Self::StaleInheritedInputScope,
            "inherited-unknown-input-scope" => Self::InheritedUnknownInputScope,
            "metadata-output" => Self::MetadataOutput,
            "metadata-input" => Self::MetadataInput,
            "metadata-both" => Self::MetadataBoth,
            "metadata-unknown-source" => Self::MetadataUnknownSource,
            "inherit" => Self::Inherit,
            "saved-input-owner" => Self::SavedInputOwner,
            _ => panic!("Unknown generated override case: {label}"),
        }
    }
    pub fn is_binding(self) -> bool {
        matches!(
            self,
            Self::EmittedCode(_)
                | Self::InheritedState(_)
                | Self::CodeOwner { .. }
                | Self::InputContext { .. }
                | Self::Combined { .. }
                | Self::NativeInput { .. }
                | Self::NativeParent { .. }
                | Self::BindingLibrary
                | Self::BindingMapped
                | Self::BindingCatalog
                | Self::BindingInputs
                | Self::BindingStaleInputs
                | Self::BindingInheritedInputs
                | Self::BindingMetadata
                | Self::BindingUnknownTarget
                | Self::BindingUnknownLibrary
                | Self::BindingUnmanifestedAsset
                | Self::BindingCombined
        )
    }
    pub fn native_parent_library(self) -> Option<bool> {
        match self {
            Self::NativeParent { library, .. } | Self::NativeInput { library, .. } => Some(library),
            Self::Combined {
                native_parent: true,
                ..
            } => Some(false),
            _ => None,
        }
    }
    pub fn underlay(self) -> bool {
        matches!(
            self,
            Self::InheritedState(Inherited::UnderlayScope | Inherited::UnderlayBinding)
        )
    }
}

pub(super) struct Case<'a> {
    pub style: Style,
    pub mode: Mode,
    pub label: &'a str,
    pub style_label: &'a str,
}
impl<'a> Case<'a> {
    pub fn parse(style: &'a str, mode: &'a str) -> Self {
        let style_kind = match style {
            "direct" => Style::Direct,
            "descendant" => Style::Descendant,
            "replacement" => Style::Replacement,
            "children" => Style::Children,
            "script" => Style::Script,
            "emitted-children" => Style::EmittedChildren,
            "emitted-replacement" => Style::EmittedReplacement,
            _ => panic!("Unknown generated override address: {style}"),
        };
        Self {
            style: style_kind,
            mode: Mode::parse(mode),
            label: mode,
            style_label: style,
        }
    }
}
