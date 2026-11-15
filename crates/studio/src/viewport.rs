//! Working in the 3D view: picking objects, outlining the selection, and
//! the move, rotate and scale gizmos.
//!
//! The view itself is rendered by the engine; the studio draws gizmos on
//! top with the camera's view-projection matrix and drives them with the
//! pointer ray the engine computes each frame.

use std::sync::Arc;

use araria_core::assets::AssetManifest;
use araria_core::{DataModel, InstanceId, Mat4, Quat, Vec2, Vec3};
use araria_platform::theme;
use araria_runtime::Runtime;
use egui::{Color32, Pos2, Rect, Shape, Stroke, Ui};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Move,
    Rotate,
    Scale,
}

/// Grid snapping for the gizmos.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snap {
    pub enabled: bool,
    /// Studs for move and scale.
    pub step: f32,
    /// Degrees for rotate.
    pub angle: f32,
}

impl Default for Snap {
    fn default() -> Self {
        Snap { enabled: true, step: 1.0, angle: 15.0 }
    }
}

/// Whether the pointer is over a menu, popup or dialog (anything drawn
/// above the studio's panels).
pub fn pointer_over_popup(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.pointer.interact_pos()).and_then(|p| ctx.layer_id_at(p)).is_some_and(|layer| layer.order != egui::Order::Background)
}

pub fn snap(value: f32, step: f32, enabled: bool) -> f32 {
    if enabled && step > 0.0 { (value / step).round() * step } else { value }
}

/// Parameter along the line `axis_origin + t * axis` closest to the ray.
pub fn closest_on_axis(ray_origin: Vec3, ray_dir: Vec3, axis_origin: Vec3, axis: Vec3) -> Option<f32> {
    let w = axis_origin - ray_origin;
    let (a, b, c) = (axis.dot(axis), axis.dot(ray_dir), ray_dir.dot(ray_dir));
    let (d, e) = (axis.dot(w), ray_dir.dot(w));
    let denominator = a * c - b * b;
    // Parallel to the view: dragging along it is meaningless.
    if denominator.abs() < 1e-5 {
        return None;
    }
    Some((b * e - c * d) / denominator)
}

pub fn ray_plane(ray_origin: Vec3, ray_dir: Vec3, point: Vec3, normal: Vec3) -> Option<Vec3> {
    let denominator = normal.dot(ray_dir);
    if denominator.abs() < 1e-5 {
        return None;
    }
    let t = normal.dot(point - ray_origin) / denominator;
    (t > 0.0).then(|| ray_origin + ray_dir * t)
}

/// Signed angle (radians) from `a` to `b` around `axis`.
pub fn signed_angle(a: Vec3, b: Vec3, axis: Vec3) -> f32 {
    let (a, b) = (a.normalize_or_zero(), b.normalize_or_zero());
    a.cross(b).dot(axis).atan2(a.dot(b))
}

/// The ray through a point of the view given in normalized device
/// coordinates (`-1..1`, y up).
pub fn view_ray(view_projection: Mat4, ndc: Vec2) -> (Vec3, Vec3) {
    let inverse = view_projection.inverse();
    let unproject = |z: f32| inverse.project_point3(Vec3::new(ndc.x, ndc.y, z));
    // Bevy uses reversed depth: 1 is the near plane.
    let (near, far) = (unproject(1.0), unproject(0.1));
    (near, (far - near).normalize_or_zero())
}

/// How an object's extent is stored.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Sizing {
    /// `Size` in studs (parts, spawn points).
    Size(Vec3),
    /// `Scale` multiplier of a model asset with this unscaled size.
    Scale(Vec3, Vec3),
    None,
}

/// An object moved by a gizmo, as it was when the drag started.
#[derive(Clone, Debug)]
struct Target {
    id: InstanceId,
    position: Vec3,
    rotation: Quat,
    sizing: Sizing,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Handle {
    /// Move along a world axis.
    Axis(usize),
    /// Rotate around a world axis.
    Ring(usize),
    /// Resize: local axis and side (+1 or -1).
    Face(usize, f32),
    /// Drag the object itself across the horizontal plane.
    Free,
}

#[derive(Clone, Debug)]
struct Drag {
    handle: Handle,
    pivot: Vec3,
    /// Direction of the handle's axis in world space.
    axis: Vec3,
    start_param: f32,
    start_point: Vec3,
    targets: Vec<Target>,
    /// Whether anything moved yet (a click without movement is a selection).
    moved: bool,
}

/// The objects a selection moves: parts directly, and every part inside a
/// selected model or folder.
fn targets(dm: &DataModel, selection: &[InstanceId], assets: &AssetManifest) -> Vec<Target> {
    let mut ids = Vec::new();
    for id in selection {
        let Some(instance) = dm.get(*id) else { continue };
        if instance.is_a("BasePart") || instance.is_a("Character") {
            ids.push(*id);
        } else {
            ids.extend(dm.descendants(*id).into_iter().filter(|d| dm.get(*d).is_some_and(|i| i.is_a("BasePart"))));
        }
    }
    ids.sort();
    ids.dedup();
    ids.into_iter()
        .filter_map(|id| {
            let instance = dm.get(id)?;
            let sizing = if instance.is_a("MeshPart") {
                let base = assets.models.get(instance.str("Asset")).map_or(Vec3::ONE * 2.0, |m| m.size());
                Sizing::Scale(instance.vec3("Scale"), base)
            } else if instance.is_a("BasePart") {
                Sizing::Size(instance.vec3("Size"))
            } else {
                Sizing::None
            };
            let rotation = if instance.is_a("Character") { Quat::from_rotation_y(instance.f32("Facing").to_radians()) } else { instance.quat("Rotation") };
            Some(Target { id, position: instance.vec3("Position"), rotation, sizing })
        })
        .collect()
}

/// Center and orientation the gizmo is drawn at.
fn pivot_of(dm: &DataModel, selection: &[InstanceId], targets: &[Target]) -> Option<(Vec3, Quat)> {
    if let [only] = targets {
        if selection.len() == 1 && selection[0] == only.id {
            return Some((only.position, only.rotation));
        }
    }
    let mut bounds: Option<(Vec3, Vec3)> = None;
    for id in selection {
        if let Some((lo, hi)) = dm.bounds(*id) {
            bounds = Some(bounds.map_or((lo, hi), |(a, b)| (a.min(lo), b.max(hi))));
        }
    }
    bounds.map(|(lo, hi)| ((lo + hi) * 0.5, Quat::IDENTITY))
}

const AXIS_COLORS: [Color32; 3] = [Color32::from_rgb(240, 82, 92), Color32::from_rgb(110, 214, 98), Color32::from_rgb(76, 150, 255)];

fn unit(axis: usize) -> Vec3 {
    [Vec3::X, Vec3::Y, Vec3::Z][axis]
}

/// Distance from a point to a segment, in pixels.
fn segment_distance(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let (ab, ap) = (b - a, p - a);
    let t = (ap.dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    (a + ab * t).distance(p)
}

/// What the viewport did this frame.
#[derive(Default)]
pub struct ViewportOutput {
    /// The user clicked an object (or empty space: `Some(None)`).
    pub clicked: Option<Option<InstanceId>>,
    /// A gizmo drag finished and changed the scene.
    pub finished_edit: Option<&'static str>,
    /// The pointer is on a gizmo or dragging one, so the game should not
    /// see the mouse.
    pub busy: bool,
    pub shift: bool,
    pub alt: bool,
}

pub struct Viewport {
    pub tool: Tool,
    pub snap: Snap,
    drag: Option<Drag>,
    pub assets: Arc<AssetManifest>,
}

impl Viewport {
    pub fn new(assets: Arc<AssetManifest>) -> Self {
        Viewport { tool: Tool::Move, snap: Snap::default(), drag: None, assets }
    }

    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Draws outlines and gizmos into `rect` and handles pointer input there.
    #[allow(clippy::too_many_lines)]
    pub fn show(&mut self, ui: &Ui, rect: Rect, runtime: &mut Runtime, selection: &[InstanceId]) -> ViewportOutput {
        let mut out = ViewportOutput::default();
        let (view_projection, ray) = {
            let input = runtime.input.borrow();
            (input.view_projection, input.mouse_ray)
        };
        let Some(view_projection) = view_projection else { return out };
        let project = |world: Vec3| -> Option<Pos2> {
            let clip = view_projection * world.extend(1.0);
            if clip.w <= 1e-4 {
                return None;
            }
            let ndc = clip.truncate() / clip.w;
            Some(Pos2::new(rect.left() + (ndc.x + 1.0) * 0.5 * rect.width(), rect.top() + (1.0 - ndc.y) * 0.5 * rect.height()))
        };
        let painter = ui.painter_at(rect);
        let pointer = ui.input(|i| i.pointer.hover_pos()).filter(|p| rect.contains(*p));
        let (pressed, released, down) = ui.input(|i| (i.pointer.primary_pressed(), i.pointer.primary_released(), i.pointer.primary_down()));
        out.shift = ui.input(|i| i.modifiers.shift);
        out.alt = ui.input(|i| i.modifiers.alt);
        let over_ui = pointer_over_popup(ui.ctx());

        let dm_ref = runtime.dm.borrow();
        let dm = &*dm_ref;
        let targets_now = targets(dm, selection, &self.assets);

        // ---- selection outlines
        for target in targets_now.iter().take(200) {
            let Some(instance) = dm.get(target.id) else { continue };
            let half = match target.sizing {
                Sizing::Size(size) => size * 0.5,
                Sizing::Scale(scale, base) => scale * base * 0.5,
                Sizing::None => Vec3::new(0.45, 0.95, 0.45) * instance.f32("Scale").max(0.1),
            };
            let center = if instance.is_a("Character") { target.position + Vec3::Y * half.y } else { target.position };
            let corners: Vec<Option<Pos2>> = (0..8)
                .map(|i| {
                    let local = Vec3::new(if i & 1 == 0 { -half.x } else { half.x }, if i & 2 == 0 { -half.y } else { half.y }, if i & 4 == 0 { -half.z } else { half.z });
                    project(center + target.rotation * local)
                })
                .collect();
            for (a, b) in [(0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5), (2, 6), (3, 7)] {
                if let (Some(a), Some(b)) = (corners[a], corners[b]) {
                    painter.line_segment([a, b], Stroke::new(1.5, theme::ACCENT_2));
                }
            }
        }
        // Non-part selections (models) also get their overall bounds.
        for id in selection {
            if dm.get(*id).is_some_and(|i| !i.is_a("BasePart") && !i.is_a("Character")) {
                if let Some((lo, hi)) = dm.bounds(*id) {
                    let corners: Vec<Option<Pos2>> = (0..8).map(|i| project(Vec3::new(if i & 1 == 0 { lo.x } else { hi.x }, if i & 2 == 0 { lo.y } else { hi.y }, if i & 4 == 0 { lo.z } else { hi.z }))).collect();
                    for (a, b) in [(0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5), (2, 6), (3, 7)] {
                        if let (Some(a), Some(b)) = (corners[a], corners[b]) {
                            painter.add(Shape::dashed_line(&[a, b], Stroke::new(1.2, theme::ACCENT_HI), 6.0, 4.0));
                        }
                    }
                }
            }
        }

        // ---- gizmo
        let pivot = self.drag.as_ref().map(|d| (d.pivot, Quat::IDENTITY)).or_else(|| pivot_of(dm, selection, &targets_now));
        let camera = view_ray(view_projection, Vec2::ZERO).0;
        let mut hovered: Option<Handle> = None;
        let mut handles: Vec<(Handle, Vec3)> = Vec::new();
        if let (Some((pivot, rotation)), false) = (pivot, targets_now.is_empty()) {
            let length = camera.distance(pivot).max(1.0) * 0.16;
            let origin = project(pivot);
            match self.tool {
                Tool::Select => {}
                Tool::Move => {
                    for axis in 0..3 {
                        let direction = unit(axis);
                        let (Some(a), Some(b)) = (origin, project(pivot + direction * length)) else { continue };
                        if let Some(p) = pointer {
                            if segment_distance(p, a, b) < 9.0 {
                                hovered = Some(Handle::Axis(axis));
                            }
                        }
                        handles.push((Handle::Axis(axis), direction));
                    }
                }
                Tool::Scale => {
                    for axis in 0..3 {
                        for side in [1.0, -1.0] {
                            let direction = rotation * unit(axis) * side;
                            if let Some(tip) = project(pivot + direction * length) {
                                if pointer.is_some_and(|p| p.distance(tip) < 10.0) {
                                    hovered = Some(Handle::Face(axis, side));
                                }
                            }
                            handles.push((Handle::Face(axis, side), direction));
                        }
                    }
                }
                Tool::Rotate => {
                    for axis in 0..3 {
                        let normal = unit(axis);
                        let (u, v) = (normal.any_orthonormal_vector(), normal.cross(normal.any_orthonormal_vector()));
                        let points: Vec<Option<Pos2>> = (0..=64).map(|i| {
                            let angle = i as f32 / 64.0 * std::f32::consts::TAU;
                            project(pivot + (u * angle.cos() + v * angle.sin()) * length)
                        }).collect();
                        if let Some(p) = pointer {
                            if points.windows(2).any(|w| matches!((w[0], w[1]), (Some(a), Some(b)) if segment_distance(p, a, b) < 7.0)) {
                                hovered = Some(Handle::Ring(axis));
                            }
                        }
                        handles.push((Handle::Ring(axis), normal));
                    }
                }
            }
            // Draw.
            let active = self.drag.as_ref().map(|d| d.handle).or(hovered);
            let color_for = |handle: Handle, axis: usize| if active == Some(handle) { Color32::from_rgb(255, 230, 90) } else { AXIS_COLORS[axis] };
            if let Some(o) = origin {
                for (handle, direction) in &handles {
                    match *handle {
                        Handle::Axis(axis) => {
                            if let Some(tip) = project(pivot + *direction * length) {
                                let color = color_for(*handle, axis);
                                painter.line_segment([o, tip], Stroke::new(3.0, color));
                                let back = (o - tip).normalized() * 12.0;
                                let side = egui::vec2(-back.y, back.x) * 0.45;
                                painter.add(Shape::convex_polygon(vec![tip, tip + back + side, tip + back - side], color, Stroke::NONE));
                            }
                        }
                        Handle::Face(axis, _) => {
                            if let Some(tip) = project(pivot + *direction * length) {
                                let color = color_for(*handle, axis);
                                painter.line_segment([o, tip], Stroke::new(2.0, theme::with_alpha(color, 0.7)));
                                painter.rect_filled(Rect::from_center_size(tip, egui::vec2(11.0, 11.0)), 2.0, color);
                            }
                        }
                        Handle::Ring(axis) => {
                            let normal = *direction;
                            let (u, v) = (normal.any_orthonormal_vector(), normal.cross(normal.any_orthonormal_vector()));
                            let points: Vec<Pos2> = (0..=64).filter_map(|i| {
                                let angle = i as f32 / 64.0 * std::f32::consts::TAU;
                                project(pivot + (u * angle.cos() + v * angle.sin()) * length)
                            }).collect();
                            painter.add(Shape::line(points, Stroke::new(if active == Some(*handle) { 3.5 } else { 2.5 }, color_for(*handle, axis))));
                        }
                        Handle::Free => {}
                    }
                }
                painter.circle_filled(o, 4.0, Color32::WHITE);
            }
        }

        // ---- pointer
        let Some((ray_origin, ray_dir)) = ray else {
            drop(dm_ref);
            return out;
        };
        if pointer.is_some() && hovered.is_some() {
            out.busy = true;
        }
        if pressed && pointer.is_some() && !over_ui {
            let handle = hovered.or_else(|| {
                // Pressing on a selected object (with the select or move
                // tool) drags it across the ground plane.
                if !matches!(self.tool, Tool::Select | Tool::Move) {
                    return None;
                }
                let hit = runtime.physics.borrow().pick(ray_origin, ray_dir, 4000.0, &[])?;
                let selected = targets_now.iter().any(|t| t.id == hit.instance) || selection.iter().any(|s| dm.is_descendant_of(hit.instance, *s));
                selected.then_some(Handle::Free)
            });
            if let (Some(handle), Some((pivot, rotation))) = (handle, pivot) {
                let axis = match handle {
                    Handle::Axis(a) => unit(a),
                    Handle::Ring(a) => unit(a),
                    Handle::Face(a, side) => rotation * unit(a) * side,
                    Handle::Free => Vec3::Y,
                };
                let (start_param, start_point) = match handle {
                    Handle::Axis(_) | Handle::Face(..) => (closest_on_axis(ray_origin, ray_dir, pivot, axis).unwrap_or(0.0), pivot),
                    Handle::Ring(_) | Handle::Free => (0.0, ray_plane(ray_origin, ray_dir, pivot, axis).unwrap_or(pivot)),
                };
                self.drag = Some(Drag { handle, pivot, axis, start_param, start_point, targets: targets_now.clone(), moved: false });
                out.busy = true;
            }
        }
        drop(dm_ref);

        if let Some(drag) = &mut self.drag {
            out.busy = true;
            if down {
                let snap = self.snap;
                let mut dm = runtime.dm.borrow_mut();
                match drag.handle {
                    Handle::Axis(_) => {
                        if let Some(t) = closest_on_axis(ray_origin, ray_dir, drag.pivot, drag.axis) {
                            let delta = snap_value(t - drag.start_param, snap.step, snap.enabled);
                            for target in &drag.targets {
                                let _ = dm.set(target.id, "Position", target.position + drag.axis * delta);
                            }
                            drag.moved |= delta != 0.0;
                        }
                    }
                    Handle::Free => {
                        if let Some(hit) = ray_plane(ray_origin, ray_dir, drag.start_point, Vec3::Y) {
                            let raw = hit - drag.start_point;
                            let delta = Vec3::new(snap_value(raw.x, snap.step, snap.enabled), 0.0, snap_value(raw.z, snap.step, snap.enabled));
                            for target in &drag.targets {
                                let _ = dm.set(target.id, "Position", target.position + delta);
                            }
                            drag.moved |= delta != Vec3::ZERO;
                        }
                    }
                    Handle::Ring(_) => {
                        if let Some(hit) = ray_plane(ray_origin, ray_dir, drag.pivot, drag.axis) {
                            let angle = signed_angle(drag.start_point - drag.pivot, hit - drag.pivot, drag.axis);
                            let angle = snap_value(angle.to_degrees(), snap.angle, snap.enabled).to_radians();
                            let turn = Quat::from_axis_angle(drag.axis, angle);
                            for target in &drag.targets {
                                let position = drag.pivot + turn * (target.position - drag.pivot);
                                let _ = dm.set(target.id, "Position", position);
                                if dm.get(target.id).is_some_and(|i| i.is_a("Character")) {
                                    let facing = araria_core::geometry::yaw_toward(turn * target.rotation * Vec3::Z);
                                    let _ = dm.set(target.id, "Facing", f64::from(facing));
                                } else {
                                    let _ = dm.set(target.id, "Rotation", (turn * target.rotation).normalize());
                                }
                            }
                            drag.moved |= angle != 0.0;
                        }
                    }
                    Handle::Face(axis, side) => {
                        if let Some(t) = closest_on_axis(ray_origin, ray_dir, drag.pivot, drag.axis) {
                            let delta = snap_value(t - drag.start_param, snap.step, snap.enabled);
                            for target in &drag.targets {
                                let direction = target.rotation * unit(axis) * side;
                                match target.sizing {
                                    Sizing::Size(size) => {
                                        let mut new_size = size;
                                        new_size[axis] = (size[axis] + delta).max(0.05);
                                        let grown = new_size[axis] - size[axis];
                                        let _ = dm.set(target.id, "Size", new_size);
                                        let _ = dm.set(target.id, "Position", target.position + direction * grown * 0.5);
                                    }
                                    Sizing::Scale(scale, base) => {
                                        let extent = (scale[axis] * base[axis]).max(0.01);
                                        let factor = ((extent + delta) / extent).max(0.02);
                                        let mut new_scale = scale;
                                        new_scale[axis] *= factor;
                                        let _ = dm.set(target.id, "Scale", new_scale);
                                        let _ = dm.set(target.id, "Position", target.position + direction * (extent * factor - extent) * 0.5);
                                    }
                                    Sizing::None => {}
                                }
                            }
                            drag.moved |= delta != 0.0;
                        }
                    }
                }
            }
            if released || !down {
                let drag = self.drag.take().expect("dragging");
                if drag.moved {
                    out.finished_edit = Some(match drag.handle {
                        Handle::Axis(_) | Handle::Free => "Move",
                        Handle::Ring(_) => "Rotate",
                        Handle::Face(..) => "Resize",
                    });
                } else if drag.handle == Handle::Free {
                    // A click on an already-selected object.
                    out.clicked = Some(self.pick(runtime, ray_origin, ray_dir, out.alt));
                }
            }
            return out;
        }

        // A click on empty space or an object selects it.
        let clicked = ui.input(|i| i.pointer.primary_clicked());
        if clicked && pointer.is_some() && !over_ui && hovered.is_none() {
            out.clicked = Some(self.pick(runtime, ray_origin, ray_dir, out.alt));
        }
        out
    }

    /// The object under the pointer: the outermost model containing the hit
    /// part, or the part itself with Alt held. Locked parts are skipped.
    fn pick(&self, runtime: &Runtime, origin: Vec3, direction: Vec3, exact: bool) -> Option<InstanceId> {
        let dm = runtime.dm.borrow();
        let physics = runtime.physics.borrow();
        let mut ignore = Vec::new();
        for _ in 0..16 {
            let hit = physics.pick(origin, direction, 4000.0, &ignore)?;
            let locked = dm.get(hit.instance).is_some_and(|i| i.is_a("BasePart") && i.bool("Locked"));
            if locked {
                ignore.push(hit.instance);
                continue;
            }
            if exact {
                return Some(hit.instance);
            }
            let mut chosen = hit.instance;
            let mut current = dm.get(hit.instance).and_then(|i| i.parent);
            while let Some(id) = current {
                let Some(instance) = dm.get(id) else { break };
                if instance.is_a("Model") || instance.is_a("Character") {
                    chosen = id;
                }
                current = instance.parent;
            }
            return Some(chosen);
        }
        None
    }

    pub fn cancel(&mut self) {
        self.drag = None;
    }
}

fn snap_value(value: f32, step: f32, enabled: bool) -> f32 {
    snap(value, step, enabled)
}

/// Where to put a new object: on the surface in the middle of the view, or
/// a little in front of the camera if nothing is there.
pub fn placement(runtime: &Runtime, size: Vec3) -> Vec3 {
    let Some(view_projection) = runtime.input.borrow().view_projection else { return Vec3::new(0.0, size.y * 0.5, 0.0) };
    let (origin, direction) = view_ray(view_projection, Vec2::ZERO);
    match runtime.physics.borrow().pick(origin, direction, 400.0, &[]) {
        Some(hit) => {
            let lift = (hit.normal * size * 0.5).abs().max_element().max(size.y * 0.5 * hit.normal.y.abs());
            let position = hit.position + hit.normal * lift;
            Vec3::new(position.x.round(), (position.y * 2.0).round() / 2.0, position.z.round())
        }
        None => (origin + direction * 20.0).round(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axis_and_plane_math() {
        // A ray looking down -Z from (3, 0, 10) passes x = 3 on the X axis.
        let t = closest_on_axis(Vec3::new(3.0, 0.0, 10.0), Vec3::NEG_Z, Vec3::ZERO, Vec3::X).unwrap();
        assert!((t - 3.0).abs() < 1e-4, "{t}");
        assert!(closest_on_axis(Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::X).is_none(), "parallel");
        let hit = ray_plane(Vec3::new(1.0, 5.0, 2.0), Vec3::NEG_Y, Vec3::ZERO, Vec3::Y).unwrap();
        assert!((hit - Vec3::new(1.0, 0.0, 2.0)).length() < 1e-5);
        assert!(ray_plane(Vec3::new(0.0, 5.0, 0.0), Vec3::Y, Vec3::ZERO, Vec3::Y).is_none(), "behind");
        let angle = signed_angle(Vec3::X, Vec3::Z, Vec3::Y);
        assert!((angle + std::f32::consts::FRAC_PI_2).abs() < 1e-5, "{angle}");
        assert_eq!(snap(2.4, 1.0, true), 2.0);
        assert_eq!(snap(2.4, 0.5, true), 2.5);
        assert_eq!(snap(2.4, 1.0, false), 2.4);
    }

    #[test]
    fn view_rays_come_from_the_camera() {
        let projection = Mat4::perspective_infinite_reverse_rh(1.2, 1.5, 0.1);
        let view = Mat4::look_at_rh(Vec3::new(0.0, 10.0, 20.0), Vec3::ZERO, Vec3::Y);
        let (origin, direction) = view_ray(projection * view, Vec2::ZERO);
        assert!((origin - Vec3::new(0.0, 10.0, 20.0)).length() < 0.2, "{origin:?}");
        assert!((direction - (Vec3::ZERO - Vec3::new(0.0, 10.0, 20.0)).normalize()).length() < 1e-3, "{direction:?}");
    }

    #[test]
    fn selections_move_their_parts() {
        let mut dm = DataModel::new();
        let model = dm.create("Model", Some(dm.workspace())).unwrap();
        let a = dm.create("Part", Some(model)).unwrap();
        let b = dm.create("Part", Some(model)).unwrap();
        dm.set(b, "Position", Vec3::new(4.0, 0.0, 0.0)).unwrap();
        let assets = AssetManifest::default();
        let found = targets(&dm, &[model], &assets);
        assert_eq!(found.iter().map(|t| t.id).collect::<Vec<_>>(), vec![a, b]);
        let (pivot, _) = pivot_of(&dm, &[model], &found).unwrap();
        assert!((pivot - Vec3::new(2.0, 0.0, 0.0)).length() < 1e-5, "{pivot:?}");
        // A single part pivots on itself, with its own orientation.
        let single = targets(&dm, &[b], &assets);
        assert_eq!(pivot_of(&dm, &[b], &single).unwrap().0, Vec3::new(4.0, 0.0, 0.0));
    }
}
