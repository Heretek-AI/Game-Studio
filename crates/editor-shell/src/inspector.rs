use serde::{Deserialize, Serialize};

/// Inspector field type (spec 04 §3). Every float row mandates numeric entry
/// + drag + snapping (gizmo parity from the `egui-gizmo` plan).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldType {
    F32,
    Vec3,
    Quat,
    U32,
    Bool,
    Text,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InspectorField {
    pub name: &'static str,
    pub ty: FieldType,
    /// Optional numeric range clamp.
    pub min: Option<f32>,
    /// Optional numeric range clamp.
    pub max: Option<f32>,
    /// Drag step + snapping quantum (0.01 unit / 0.1° precision handles).
    pub step: f32,
    /// Whether drag gestures are supported on this row.
    pub draggable: bool,
    /// Whether typed numeric entry is supported on this row.
    pub numeric_entry: bool,
    /// Whether grid/angle snapping applies.
    pub snapping: bool,
}

fn float_row(name: &'static str, min: Option<f32>, max: Option<f32>, step: f32) -> InspectorField {
    InspectorField {
        name,
        ty: FieldType::F32,
        min,
        max,
        step,
        draggable: true,
        numeric_entry: true,
        snapping: true,
    }
}

fn vec3_row(name: &'static str, step: f32) -> InspectorField {
    InspectorField {
        name,
        ty: FieldType::Vec3,
        min: None,
        max: None,
        step,
        draggable: true,
        numeric_entry: true,
        snapping: true,
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComponentSchema {
    pub component: &'static str,
    pub fields: Vec<InspectorField>,
    pub schema_file: &'static str,
}

pub fn transform_fields() -> Vec<InspectorField> {
    vec![
        vec3_row("translation", 0.01),
        InspectorField {
            name: "rotation",
            ty: FieldType::Quat,
            min: None,
            max: None,
            step: 0.1,
            draggable: true,
            numeric_entry: true,
            snapping: true,
        },
        vec3_row("scale", 0.01),
    ]
}

pub fn velocity_fields() -> Vec<InspectorField> {
    vec![vec3_row("0", 0.01)]
}

pub fn body_handle_fields() -> Vec<InspectorField> {
    vec![InspectorField {
        name: "0",
        ty: FieldType::U32,
        min: Some(0.0),
        max: None,
        step: 1.0,
        draggable: false,
        numeric_entry: true,
        snapping: false,
    }]
}

pub fn static_fields() -> Vec<InspectorField> {
    vec![InspectorField {
        name: "present",
        ty: FieldType::Bool,
        min: None,
        max: None,
        step: 1.0,
        draggable: false,
        numeric_entry: false,
        snapping: false,
    }]
}

pub fn name_fields() -> Vec<InspectorField> {
    vec![
        float_row("placeholder", None, None, 0.01),
        InspectorField {
            name: "0",
            ty: FieldType::Text,
            min: None,
            max: None,
            step: 1.0,
            draggable: false,
            numeric_entry: true,
            snapping: false,
        },
    ]
    .into_iter()
    .skip(1)
    .collect()
}

pub fn script_ref_fields() -> Vec<InspectorField> {
    vec![InspectorField {
        name: "0",
        ty: FieldType::U32,
        min: Some(0.0),
        max: None,
        step: 1.0,
        draggable: false,
        numeric_entry: true,
        snapping: false,
    }]
}

pub fn render_id_fields() -> Vec<InspectorField> {
    vec![InspectorField {
        name: "0",
        ty: FieldType::U32,
        min: Some(0.0),
        max: None,
        step: 1.0,
        draggable: false,
        numeric_entry: true,
        snapping: false,
    }]
}

/// Registry for all Phase-1 components (spec 01 §3 frozen set). Every
/// inspector frontend (`egui`, Tauri/React, CLI) is driven by this registry +
/// `schemas/component.*.json`, so they can never disagree.
pub fn all_components() -> Vec<ComponentSchema> {
    vec![
        ComponentSchema {
            component: "Transform",
            fields: transform_fields(),
            schema_file: "schemas/component.Transform.json",
        },
        ComponentSchema {
            component: "Velocity",
            fields: velocity_fields(),
            schema_file: "schemas/component.Velocity.json",
        },
        ComponentSchema {
            component: "BodyHandle",
            fields: body_handle_fields(),
            schema_file: "schemas/component.BodyHandle.json",
        },
        ComponentSchema {
            component: "Static",
            fields: static_fields(),
            schema_file: "schemas/component.Static.json",
        },
        ComponentSchema {
            component: "Name",
            fields: name_fields(),
            schema_file: "schemas/component.Name.json",
        },
        ComponentSchema {
            component: "ScriptRef",
            fields: script_ref_fields(),
            schema_file: "schemas/component.ScriptRef.json",
        },
        ComponentSchema {
            component: "RenderId",
            fields: render_id_fields(),
            schema_file: "schemas/component.RenderId.json",
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_covers_all_phase1_components() {
        let names: Vec<_> = all_components().iter().map(|c| c.component).collect();
        for expected in [
            "Transform",
            "Velocity",
            "BodyHandle",
            "Static",
            "Name",
            "ScriptRef",
            "RenderId",
        ] {
            assert!(names.contains(&expected), "missing {expected}");
        }
    }

    #[test]
    fn float_rows_have_entry_drag_snap() {
        for component in all_components() {
            for field in &component.fields {
                match field.ty {
                    FieldType::F32 | FieldType::Vec3 | FieldType::Quat => {
                        assert!(
                            field.numeric_entry,
                            "{}::{} numeric",
                            component.component, field.name
                        );
                        assert!(
                            field.draggable,
                            "{}::{} drag",
                            component.component, field.name
                        );
                        assert!(
                            field.snapping,
                            "{}::{} snap",
                            component.component, field.name
                        );
                        assert!(field.step > 0.0);
                    }
                    _ => {}
                }
            }
        }
    }

    #[test]
    fn transform_matches_schema_shape() {
        let fields = transform_fields();
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].name, "translation");
        assert_eq!(fields[0].step, 0.01);
        assert_eq!(fields[1].name, "rotation");
        assert_eq!(fields[1].step, 0.1);
    }
}
