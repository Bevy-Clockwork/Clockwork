//! Surface helpers for the Workshop. The overlay drawing itself is shared with the agent and
//! lives in `clockwork_recipe::preview`.

use clockwork_core::surface::SurfaceProperty;

pub use clockwork_recipe::preview::{LEGEND, overlay};

/// A short readable form of a property, for the Inspector.
pub fn describe(property: &SurfaceProperty) -> String {
    match property {
        SurfaceProperty::SolidParts(mask) => {
            let parts: Vec<&str> = [
                (1, "top left"),
                (2, "top right"),
                (4, "bottom left"),
                (8, "bottom right"),
            ]
            .iter()
            .filter(|(bit, _)| mask & bit != 0)
            .map(|(_, name)| *name)
            .collect();
            format!("Solid parts: {}", parts.join(", "))
        }
        SurfaceProperty::Extension(id) => id.clone(),
        other => format!("{other:?}"),
    }
}
