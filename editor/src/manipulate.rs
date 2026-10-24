//! Direct manipulation in the viewport: click to select (Ctrl or Shift adds), and move, rotate
//! and scale the selection with gizmos, with optional snapping. Hotkeys: W move, E rotate,
//! R scale, G snapping.
//!
//! Picking casts the cursor ray against the scene's meshes and walks up from the entity it hits to
//! the scene node it belongs to. Gizmos are drawn with Bevy's immediate-mode gizmos; their handles
//! are hit-tested with plain ray maths, so nothing needs to be spawned.

use bevy::{
    math::Affine3A,
    picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings},
    prelude::*,
    window::PrimaryWindow,
};

use crate::{
    ViewportCamera,
    layers::LayerPlane,
    scene::{NodeId, ROOT, Scene},
    state::EditorState,
    terrains::SceneTerrains,
    workshop::{OrbitCamera, SceneNode},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    #[default]
    Move,
    Rotate,
    Scale,
}

/// Snapping steps: meters, degrees, and a multiple for scale.
pub const SNAP_MOVE: f32 = 0.5;
pub const SNAP_ROTATE: f32 = 15.0;
pub const SNAP_SCALE: f32 = 0.05;

/// A mouse press this many pixels or fewer from where it was released is a click.
const CLICK_SLOP: f32 = 4.0;

const X_COLOR: Color = Color::srgb(0.95, 0.35, 0.3);
const Y_COLOR: Color = Color::srgb(0.45, 0.85, 0.35);
const Z_COLOR: Color = Color::srgb(0.35, 0.55, 0.95);
const GROUND_COLOR: Color = Color::srgb(0.95, 0.8, 0.35);
const SELECT_COLOR: Color = Color::srgb(0.89, 0.75, 0.42);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handle {
    Axis(usize),
    Ground,
    Ring,
}

/// A node being manipulated, as it was when the drag began.
struct Original {
    id: NodeId,
    yaw: f32,
    scale: f32,
    world: Vec3,
    /// The parent's world transform, inverted: world positions become local ones.
    parent_inverse: Affine3A,
    /// Whether the node sits on the ground (characters and objects), to stay on terrain.
    on_ground: bool,
}

struct Drag {
    handle: Handle,
    pivot: Vec3,
    /// Where on the handle the drag began (a distance along the axis, or a point on the ground).
    start: Vec3,
    start_param: f32,
    originals: Vec<Original>,
}

#[derive(Resource, Default)]
pub struct Manipulation {
    drag: Option<Drag>,
    /// Where the left button went down in the viewport, if it did and nothing took it.
    press: Option<Vec2>,
    hover: Option<Handle>,
}

fn axis_dir(i: usize) -> Vec3 {
    [Vec3::X, Vec3::Y, Vec3::Z][i]
}

fn axis_color(i: usize) -> Color {
    [X_COLOR, Y_COLOR, Z_COLOR][i]
}

/// The point on the line `origin + t * dir` nearest to the ray, and `t` there.
fn closest_on_line(ray: Ray3d, origin: Vec3, dir: Vec3) -> (f32, f32) {
    let d1 = Vec3::from(ray.direction);
    let w = ray.origin - origin;
    let (a, b, c) = (d1.dot(d1), d1.dot(dir), dir.dot(dir));
    let (d, e) = (d1.dot(w), dir.dot(w));
    let denom = a * c - b * b;
    if denom.abs() < 1e-6 {
        return (0.0, f32::MAX);
    }
    let s = (b * e - c * d) / denom;
    let t = (a * e - b * d) / denom;
    let on_ray = ray.origin + d1 * s.max(0.0);
    let on_line = origin + dir * t;
    (t, on_ray.distance(on_line))
}

/// Where the ray meets the horizontal plane at `height`.
fn ground_hit(ray: Ray3d, height: f32) -> Option<Vec3> {
    let d = ray.intersect_plane(Vec3::new(0.0, height, 0.0), InfinitePlane3d::new(Vec3::Y))?;
    Some(ray.get_point(d))
}

fn snapped(value: f32, step: f32) -> f32 {
    (value / step).round() * step
}

/// The handle under the ray, if any, for a gizmo of `size` meters at `pivot`.
fn hit_handle(tool: Tool, ray: Ray3d, pivot: Vec3, size: f32) -> Option<Handle> {
    let reach = size * 0.09;
    match tool {
        Tool::Move => {
            // The ground square first (it sits near the pivot), then the axes.
            if let Some(hit) = ground_hit(ray, pivot.y) {
                let d = hit - pivot;
                if (size * 0.18..=size * 0.5).contains(&d.x)
                    && (size * 0.18..=size * 0.5).contains(&d.z)
                {
                    return Some(Handle::Ground);
                }
            }
            (0..3)
                .filter_map(|i| {
                    let (t, distance) = closest_on_line(ray, pivot, axis_dir(i));
                    (distance <= reach && (0.0..=size * 1.05).contains(&t)).then_some((i, distance))
                })
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| Handle::Axis(i))
        }
        Tool::Rotate | Tool::Scale => {
            let hit = ground_hit(ray, pivot.y)?;
            ((hit - pivot).length() - size * 0.9)
                .abs()
                .le(&(reach * 1.4))
                .then_some(Handle::Ring)
        }
    }
}

/// The scene node an entity belongs to: the entity itself or the nearest ancestor with one.
fn node_of(entity: Entity, nodes: &Query<&SceneNode>, parents: &Query<&ChildOf>) -> Option<NodeId> {
    let mut current = entity;
    loop {
        if let Ok(node) = nodes.get(current) {
            return Some(node.0);
        }
        current = parents.get(current).ok()?.parent();
    }
}

#[allow(clippy::too_many_arguments)] // Bevy systems take their dependencies as parameters.
pub fn manipulate_system(
    mut state: ResMut<EditorState>,
    mut manipulation: ResMut<Manipulation>,
    terrains: Res<SceneTerrains>,
    orbit: Res<OrbitCamera>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    window: Single<&Window, With<PrimaryWindow>>,
    camera: Single<(&Camera, &GlobalTransform), With<ViewportCamera>>,
    mut ray_cast: MeshRayCast,
    nodes: Query<&SceneNode>,
    layer_planes: Query<(), With<LayerPlane>>,
    parents: Query<&ChildOf>,
    mut gizmos: Gizmos,
) {
    state.workshop.gizmo_busy = false;
    if state.workshop.playing.is_some() {
        *manipulation = Manipulation::default();
        return;
    }

    // Hotkeys.
    if !state.workshop.typing && !keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]) {
        for (key, tool) in [
            (KeyCode::KeyW, Tool::Move),
            (KeyCode::KeyE, Tool::Rotate),
            (KeyCode::KeyR, Tool::Scale),
        ] {
            if keys.just_pressed(key) && state.workshop.viewport_hovered {
                state.workshop.tool = tool;
            }
        }
        if keys.just_pressed(KeyCode::KeyG) && state.workshop.viewport_hovered {
            state.workshop.snap = !state.workshop.snap;
        }
    }

    let tool = state.workshop.tool;
    let snap = state.workshop.snap;
    let (camera, camera_transform) = *camera;
    let ray = window
        .cursor_position()
        .and_then(|c| camera.viewport_to_world(camera_transform, c).ok());

    // What the gizmo works on: the selection without locked nodes and without nodes below other
    // selected ones. Its pivot is the primary node.
    let selection: Vec<NodeId> = {
        let ws = &state.workshop;
        ws.scene
            .top_level(&ws.selection())
            .into_iter()
            .filter(|id| ws.scene.get(*id).is_some_and(|n| !n.locked))
            .collect()
    };
    let pivot = state
        .workshop
        .selected_node
        .filter(|p| selection.contains(p))
        .or(selection.last().copied())
        .map(|id| state.workshop.scene.world_transform(id).translation);
    let size = orbit.distance * 0.16;

    // Selection marks.
    for id in state.workshop.selection() {
        if id == ROOT {
            continue;
        }
        let at = state.workshop.scene.world_transform(id).translation;
        let primary = state.workshop.selected_node == Some(id);
        gizmos.circle(
            Isometry3d::new(
                at + Vec3::Y * 0.03,
                Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
            ),
            if primary { 0.5 } else { 0.35 },
            if primary {
                SELECT_COLOR
            } else {
                SELECT_COLOR.with_alpha(0.55)
            },
        );
    }

    // Handles under the cursor.
    let hovering_viewport = state.workshop.viewport_hovered;
    manipulation.hover = None;
    if manipulation.drag.is_none()
        && hovering_viewport
        && let (Some(ray), Some(pivot)) = (ray, pivot)
    {
        manipulation.hover = hit_handle(tool, ray, pivot, size);
    }
    state.workshop.gizmo_busy = manipulation.hover.is_some() || manipulation.drag.is_some();

    // A pick asked for by the remote control.
    if let Some(at) = state.workshop.pick_request.take()
        && let Ok(ray) = camera.viewport_to_world(camera_transform, at)
    {
        pick(
            &mut state,
            &mut ray_cast,
            (&nodes, &layer_planes),
            &parents,
            ray,
            false,
        );
    }

    // Press, drag, release.
    if buttons.just_pressed(MouseButton::Left) && hovering_viewport {
        if let (Some(handle), Some(ray), Some(pivot)) = (manipulation.hover, ray, pivot) {
            manipulation.drag = start_drag(&state, &terrains, handle, tool, ray, pivot, &selection);
        } else {
            manipulation.press = window.cursor_position();
        }
    }
    if let (Some(drag), Some(ray)) = (&manipulation.drag, ray)
        && buttons.pressed(MouseButton::Left)
    {
        apply_drag(&mut state.workshop.scene, &terrains, drag, tool, snap, ray);
    }
    if buttons.just_released(MouseButton::Left) {
        manipulation.drag = None;
        if let (Some(down), Some(up)) = (manipulation.press.take(), window.cursor_position())
            && down.distance(up) <= CLICK_SLOP
            && let Some(ray) = ray
        {
            let additive = keys.any_pressed([
                KeyCode::ControlLeft,
                KeyCode::ControlRight,
                KeyCode::ShiftLeft,
                KeyCode::ShiftRight,
            ]);
            pick(
                &mut state,
                &mut ray_cast,
                (&nodes, &layer_planes),
                &parents,
                ray,
                additive,
            );
        }
    }
    if !buttons.pressed(MouseButton::Left) {
        manipulation.drag = None;
    }

    // The gizmo.
    if let Some(pivot) = pivot {
        let active = manipulation
            .drag
            .as_ref()
            .map(|d| d.handle)
            .or(manipulation.hover);
        draw_gizmo(&mut gizmos, tool, pivot, size, active);
    }
}

fn draw_gizmo(gizmos: &mut Gizmos, tool: Tool, pivot: Vec3, size: f32, active: Option<Handle>) {
    let bright = |color: Color, on: bool| if on { Color::WHITE } else { color };
    match tool {
        Tool::Move => {
            for i in 0..3 {
                let on = active == Some(Handle::Axis(i));
                gizmos
                    .arrow(pivot, pivot + axis_dir(i) * size, bright(axis_color(i), on))
                    .with_tip_length(size * 0.18);
            }
            let (a, b) = (size * 0.18, size * 0.5);
            let on = active == Some(Handle::Ground);
            gizmos.linestrip(
                [
                    pivot + Vec3::new(a, 0.0, a),
                    pivot + Vec3::new(b, 0.0, a),
                    pivot + Vec3::new(b, 0.0, b),
                    pivot + Vec3::new(a, 0.0, b),
                    pivot + Vec3::new(a, 0.0, a),
                ],
                bright(GROUND_COLOR, on),
            );
        }
        Tool::Rotate | Tool::Scale => {
            let color = if tool == Tool::Rotate {
                Y_COLOR
            } else {
                GROUND_COLOR
            };
            let on = active == Some(Handle::Ring);
            gizmos.circle(
                Isometry3d::new(pivot, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
                size * 0.9,
                bright(color, on),
            );
            if tool == Tool::Scale {
                gizmos.circle(
                    Isometry3d::new(pivot, Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
                    size * 0.9 * 0.6,
                    color.with_alpha(0.35),
                );
            }
        }
    }
}

fn start_drag(
    state: &EditorState,
    terrains: &SceneTerrains,
    handle: Handle,
    tool: Tool,
    ray: Ray3d,
    pivot: Vec3,
    selection: &[NodeId],
) -> Option<Drag> {
    let _ = terrains;
    let scene = &state.workshop.scene;
    let originals = selection
        .iter()
        .filter_map(|id| {
            let node = scene.get(*id)?;
            let parent_world = node
                .parent
                .map(|p| scene.world_transform(p))
                .unwrap_or_default();
            Some(Original {
                id: *id,
                yaw: node.yaw,
                scale: node.scale,
                world: scene.world_transform(*id).translation,
                parent_inverse: parent_world.compute_affine().inverse(),
                on_ground: node.item.is_some() && state.workshop.item_drops_on_ground(node),
            })
        })
        .collect::<Vec<_>>();
    if originals.is_empty() {
        return None;
    }
    let (start, start_param) = match (tool, handle) {
        (Tool::Move, Handle::Axis(i)) => (pivot, closest_on_line(ray, pivot, axis_dir(i)).0),
        (_, _) => (ground_hit(ray, pivot.y)?, 0.0),
    };
    Some(Drag {
        handle,
        pivot,
        start,
        start_param,
        originals,
    })
}

fn apply_drag(
    scene: &mut Scene,
    terrains: &SceneTerrains,
    drag: &Drag,
    tool: Tool,
    snap: bool,
    ray: Ray3d,
) {
    // The change as a world translation, a turn about the pivot, or a scale factor.
    let mut delta = Vec3::ZERO;
    let mut turn = 0.0f32;
    let mut factor = 1.0f32;
    match (tool, drag.handle) {
        (Tool::Move, Handle::Axis(i)) => {
            let (t, _) = closest_on_line(ray, drag.pivot, axis_dir(i));
            let mut moved = t - drag.start_param;
            if snap {
                let at = drag.pivot[i] + moved;
                moved = snapped(at, SNAP_MOVE) - drag.pivot[i];
            }
            delta = axis_dir(i) * moved;
        }
        (Tool::Move, Handle::Ground) => {
            let Some(hit) = ground_hit(ray, drag.pivot.y) else {
                return;
            };
            delta = hit - drag.start;
            if snap {
                delta.x = snapped(drag.pivot.x + delta.x, SNAP_MOVE) - drag.pivot.x;
                delta.z = snapped(drag.pivot.z + delta.z, SNAP_MOVE) - drag.pivot.z;
            }
        }
        (Tool::Rotate, _) => {
            let Some(hit) = ground_hit(ray, drag.pivot.y) else {
                return;
            };
            let angle = |p: Vec3| (p.z - drag.pivot.z).atan2(p.x - drag.pivot.x);
            let mut degrees = (angle(drag.start) - angle(hit)).to_degrees();
            if snap {
                degrees = snapped(degrees, SNAP_ROTATE);
            }
            turn = degrees;
        }
        (Tool::Scale, _) => {
            let Some(hit) = ground_hit(ray, drag.pivot.y) else {
                return;
            };
            let start = (drag.start - drag.pivot).length().max(0.01);
            factor = ((hit - drag.pivot).length() / start).clamp(0.05, 20.0);
        }
        _ => {}
    }

    let rotation = Quat::from_rotation_y(turn.to_radians());
    for original in &drag.originals {
        let mut world = original.world;
        if tool == Tool::Rotate {
            world = drag.pivot + rotation * (world - drag.pivot);
        }
        world += delta;
        if original.on_ground
            && matches!(drag.handle, Handle::Ground)
            && let Some(height) = terrains.floor_below(Vec3::new(world.x, world.y + 1.0, world.z))
        {
            world.y = height;
        }
        let local = original.parent_inverse.transform_point3(world);
        let Some(node) = scene.get_mut(original.id) else {
            continue;
        };
        node.position = local.to_array();
        node.yaw = original.yaw + turn;
        node.scale = if tool == Tool::Scale {
            let scaled = original.scale * factor;
            if snap {
                snapped(scaled, SNAP_SCALE).max(SNAP_SCALE)
            } else {
                scaled
            }
        } else {
            original.scale
        };
    }
    scene.dirty = true;
}

/// Selects the node under the cursor, or nothing when the click lands on empty space. Clicks go
/// through an area's layers (a foreground over a character picks the character).
fn pick(
    state: &mut EditorState,
    ray_cast: &mut MeshRayCast,
    (nodes, layer_planes): (&Query<&SceneNode>, &Query<(), With<LayerPlane>>),
    parents: &Query<&ChildOf>,
    ray: Ray3d,
    additive: bool,
) {
    let solid = |entity: Entity| !layer_planes.contains(entity);
    let hits = ray_cast.cast_ray(ray, &MeshRayCastSettings::default().with_filter(&solid));
    let picked = hits.iter().find_map(|(entity, _)| {
        let id = node_of(*entity, nodes, parents)?;
        state
            .workshop
            .scene
            .get(id)
            .filter(|n| !n.locked && id != ROOT)
            .map(|_| id)
    });
    match picked {
        Some(id) if additive => state.workshop.toggle_select(id),
        Some(id) => {
            state.workshop.select_only(id);
            if state.workshop.adjusting != Some(id) {
                state.workshop.adjusting = None;
            }
        }
        None if !additive => {
            state.workshop.selected_node = None;
            state.workshop.selection.clear();
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ray(origin: Vec3, to: Vec3) -> Ray3d {
        Ray3d::new(origin, Dir3::new(to - origin).unwrap())
    }

    #[test]
    fn the_nearest_point_on_an_axis_is_found_along_it() {
        // A ray from above the x axis at x = 3 looking down: t = 3, distance 0.
        let (t, distance) = closest_on_line(
            ray(Vec3::new(3.0, 5.0, 0.0), Vec3::new(3.0, 0.0, 0.0)),
            Vec3::ZERO,
            Vec3::X,
        );
        assert!((t - 3.0).abs() < 1e-4 && distance < 1e-4);
        // The same ray passing 1 m beside the axis.
        let (_, distance) = closest_on_line(
            ray(Vec3::new(3.0, 5.0, 1.0), Vec3::new(3.0, 0.0, 1.0)),
            Vec3::ZERO,
            Vec3::X,
        );
        assert!((distance - 1.0).abs() < 1e-4);
    }

    #[test]
    fn handles_are_hit_where_they_are_drawn() {
        let pivot = Vec3::ZERO;
        let size = 2.0;
        let from_above = |p: Vec3| ray(p + Vec3::Y * 10.0, p);
        // Moving: the x axis, and the ground square, and nothing far away.
        assert_eq!(
            hit_handle(
                Tool::Move,
                from_above(Vec3::new(1.5, 0.0, 0.01)),
                pivot,
                size
            ),
            Some(Handle::Axis(0))
        );
        assert_eq!(
            hit_handle(
                Tool::Move,
                from_above(Vec3::new(0.7, 0.0, 0.7)),
                pivot,
                size
            ),
            Some(Handle::Ground)
        );
        assert_eq!(
            hit_handle(
                Tool::Move,
                from_above(Vec3::new(5.0, 0.0, 5.0)),
                pivot,
                size
            ),
            None
        );
        // The rotate ring is at 0.9 of the size.
        assert_eq!(
            hit_handle(
                Tool::Rotate,
                from_above(Vec3::new(1.8, 0.0, 0.0)),
                pivot,
                size
            ),
            Some(Handle::Ring)
        );
        assert_eq!(
            hit_handle(
                Tool::Rotate,
                from_above(Vec3::new(0.2, 0.0, 0.0)),
                pivot,
                size
            ),
            None
        );
    }

    fn scene_with_two() -> (Scene, Vec<NodeId>) {
        let mut scene = Scene::default();
        let a = scene.add(ROOT, "a".into(), None, Vec3::new(1.0, 0.0, 0.0));
        let b = scene.add(ROOT, "b".into(), None, Vec3::new(3.0, 0.0, 0.0));
        (scene, vec![a, b])
    }

    fn drag_for(
        scene: &Scene,
        ids: &[NodeId],
        handle: Handle,
        pivot: Vec3,
        start: Vec3,
        start_param: f32,
    ) -> Drag {
        Drag {
            handle,
            pivot,
            start,
            start_param,
            originals: ids
                .iter()
                .map(|id| {
                    let node = scene.get(*id).unwrap();
                    Original {
                        id: *id,
                        yaw: node.yaw,
                        scale: node.scale,
                        world: scene.world_transform(*id).translation,
                        parent_inverse: Affine3A::IDENTITY,
                        on_ground: false,
                    }
                })
                .collect(),
        }
    }

    #[test]
    fn dragging_an_axis_moves_every_selected_node_together() {
        let (mut scene, ids) = scene_with_two();
        let terrains = SceneTerrains::default();
        let pivot = Vec3::new(1.0, 0.0, 0.0);
        // Grab the x axis at 0.5 m along it, then drag to 2.5 m: 2 m east.
        let (start_param, _) = closest_on_line(
            ray(Vec3::new(1.5, 5.0, 0.0), Vec3::new(1.5, 0.0, 0.0)),
            pivot,
            Vec3::X,
        );
        let drag = drag_for(&scene, &ids, Handle::Axis(0), pivot, pivot, start_param);
        let now = ray(Vec3::new(3.5, 5.0, 0.0), Vec3::new(3.5, 0.0, 0.0));
        apply_drag(&mut scene, &terrains, &drag, Tool::Move, false, now);
        assert!((scene.get(ids[0]).unwrap().position[0] - 3.0).abs() < 1e-4);
        assert!((scene.get(ids[1]).unwrap().position[0] - 5.0).abs() < 1e-4);
        assert!(scene.dirty);
        // With snapping the leader lands on a 0.5 m step.
        let now = ray(Vec3::new(3.37, 5.0, 0.0), Vec3::new(3.37, 0.0, 0.0));
        apply_drag(&mut scene, &terrains, &drag, Tool::Move, true, now);
        let x = scene.get(ids[0]).unwrap().position[0];
        assert!((x / 0.5 - (x / 0.5).round()).abs() < 1e-4, "{x}");
    }

    #[test]
    fn rotating_turns_nodes_about_the_pivot_and_scaling_scales_them() {
        let (mut scene, ids) = scene_with_two();
        let terrains = SceneTerrains::default();
        let pivot = Vec3::ZERO;
        // From the +x side of the ring to the -z side: a quarter turn.
        let start = Vec3::new(2.0, 0.0, 0.0);
        let drag = drag_for(&scene, &ids, Handle::Ring, pivot, start, 0.0);
        let now = ray(Vec3::new(0.0, 5.0, -2.0), Vec3::new(0.0, 0.0, -2.0));
        apply_drag(&mut scene, &terrains, &drag, Tool::Rotate, true, now);
        let a = scene.get(ids[0]).unwrap();
        assert!((a.yaw.abs() - 90.0).abs() < 1e-3, "{}", a.yaw);
        // The node at x = 1 went round the pivot to where the turn took it, 1 m from it.
        assert!((Vec3::from(a.position).length() - 1.0).abs() < 1e-4);
        assert!(Vec3::from(a.position).x.abs() < 1e-4);

        // Twice as far from the pivot: twice the scale.
        let (mut scene, ids) = scene_with_two();
        let drag = drag_for(&scene, &ids, Handle::Ring, pivot, start, 0.0);
        let now = ray(Vec3::new(4.0, 5.0, 0.0), Vec3::new(4.0, 0.0, 0.0));
        apply_drag(&mut scene, &terrains, &drag, Tool::Scale, false, now);
        assert!((scene.get(ids[1]).unwrap().scale - 2.0).abs() < 1e-4);
        // Positions don't change when scaling.
        assert!((scene.get(ids[1]).unwrap().position[0] - 3.0).abs() < 1e-4);
    }

    #[test]
    fn snapping_rounds_to_steps() {
        assert_eq!(snapped(1.3, 0.5), 1.5);
        assert_eq!(snapped(-0.2, 0.5), 0.0);
        assert_eq!(snapped(37.0, 15.0), 30.0);
    }
}
