//! Camera assets: how the camera follows the played character. A character names its camera
//! asset (or writes one in place); without one it gets a built-in camera.

use serde::{Deserialize, Serialize};

use crate::{
    asset::{BUILTIN, OutputRef, Slot},
    character::FollowCamera,
    character2d::TopDownCamera,
    recipe::Output,
};

/// The two ways the camera follows a character.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CameraRig {
    /// Behind a 3D character, swinging round as it runs.
    Follow(FollowCamera),
    /// Above the character, north up.
    TopDown(TopDownCamera),
}

/// The built-in camera of top-down characters.
pub const TOP_DOWN: &str = "top-down";
/// The built-in camera of top-down characters in 3D worlds, where they stand upright.
pub const BILLBOARD: &str = "billboard";
/// A built-in camera behind 3D characters.
pub const FOLLOW: &str = "follow";

/// The built-in camera named `id`.
pub fn builtin(id: &str) -> OutputRef {
    OutputRef::new(BUILTIN, id)
}

/// The built-in camera named `id`, as a slot.
pub fn builtin_slot<T>(id: &str) -> Slot<T> {
    Slot::Ref(builtin(id))
}

pub(crate) fn builtins() -> Vec<Output> {
    let camera = |id: &str, rig: CameraRig| Output::Camera {
        id: id.into(),
        camera: Slot::Inline(rig),
    };
    vec![
        camera(TOP_DOWN, CameraRig::TopDown(TopDownCamera::default())),
        camera(
            BILLBOARD,
            CameraRig::TopDown(TopDownCamera {
                distance: 8.0,
                pitch: 30.0,
            }),
        ),
        camera(
            FOLLOW,
            CameraRig::Follow(FollowCamera {
                distance: 6.0,
                pitch: 20.0,
                height: 1.0,
                follow: 1.5,
            }),
        ),
    ]
}
