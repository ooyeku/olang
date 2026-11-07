//! Physics: 2D rigid bodies, colliders, joints and queries (rapier2d,
//! behind Toad's own small API), and play swarms colliding with bodies
//! and with each other through a uniform grid of their own.
//!
//! - **Units are the game's.** A world has a `scale` (world units to a
//!   metre, 50 by default); positions, sizes, speeds and gravity go in
//!   and come out in world units, so a 100-unit crate is a 2 m crate to
//!   the solver, which is tuned for metres.
//! - **Bodies and joints are numbered** by the world (the framework keys
//!   them by name). A body's colliders carry its number, so contacts and
//!   queries answer numbers.
//! - **Contacts arrive batched**: `simulate` answers every touch that
//!   started or stopped in the step, in the solver's order (the same run
//!   to run: rapier's enhanced determinism).
//! - **Swarms collide by a grid.** Thousands of entities against a few
//!   hundred colliders, or against another swarm, is a grid lookup and a
//!   few exact tests each: no solver bodies for them. Entities are pushed
//!   out, bounce, die or are marked; bodies they hit are counted and can
//!   be pushed back. The work is spread over the cores in chunks and put
//!   together in order, so the result is the same every run.

use super::cpu::Columns;
use super::kernel::{Program, Ty};
use super::{map_of, num, pair};
use crate::ast::{Value, ValueMap};
use rapier2d::control::{CharacterAutostep, CharacterLength, KinematicCharacterController};
use rapier2d::prelude::*;
#[cfg(feature = "native")]
use rayon::prelude::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// One physics world.
pub struct World {
    w: PhysicsWorld,
    /// Metres per world unit.
    k: f32,
    next: i64,
    bodies: HashMap<i64, RigidBodyHandle>,
    joints: HashMap<i64, ImpulseJointHandle>,
    /// Each moving body's pose before the last step (world units), for
    /// drawing between steps.
    prev: HashMap<i64, ([f32; 2], f32)>,
    /// One-way colliders, by the side (local) things may land on.
    one_way: HashMap<ColliderHandle, Vector>,
    /// Touches found by `warm`, told with the first step.
    pending: Vec<Touch>,
    /// Whether a walking body stood on something after its last walk.
    grounded: HashMap<i64, bool>,
}

fn v(p: (f32, f32), k: f32) -> Vector {
    Vector::new(p.0 * k, p.1 * k)
}

fn tup(xs: &[f32]) -> Value {
    Value::Tuple(Arc::new(
        xs.iter().map(|x| Value::Float(*x as f64)).collect(),
    ))
}

fn map(entries: Vec<(&str, Value)>) -> Value {
    let mut m = ValueMap::default();
    for (k, x) in entries {
        m.insert(k.into(), x);
    }
    Value::Map(Arc::new(m))
}

fn flag(m: &ValueMap, k: &str, d: bool) -> bool {
    match m.get(k) {
        Some(Value::Boolean(b)) => *b,
        _ => d,
    }
}

impl World {
    /// `opts`: `gravity` (world units/s², `(0, -980)` by default), `scale`
    /// (world units to a metre).
    pub fn new(opts: &Value) -> Result<World, String> {
        let empty = ValueMap::default();
        let m = match opts {
            Value::Unit => &empty,
            o => map_of(o, "the physics options")?,
        };
        let scale = m.get("scale").and_then(num).unwrap_or(50.0);
        if scale.is_nan() || scale <= 0.0 {
            return Err("toad.physics: `scale` (world units to a metre) is above zero".into());
        }
        let k = 1.0 / scale;
        let mut w = PhysicsWorld::new();
        w.gravity = v(m.get("gravity").and_then(pair).unwrap_or((0.0, -980.0)), k);
        Ok(World {
            w,
            k,
            next: 1,
            bodies: HashMap::new(),
            joints: HashMap::new(),
            prev: HashMap::new(),
            one_way: HashMap::new(),
            pending: Vec::new(),
            grounded: HashMap::new(),
        })
    }

    fn handle(&self, id: i64) -> Result<RigidBodyHandle, String> {
        self.bodies
            .get(&id)
            .copied()
            .ok_or_else(|| format!("toad: no body {id} in this world"))
    }

    fn id_of_collider(&self, h: ColliderHandle) -> Option<i64> {
        let c = self.w.colliders.get(h)?;
        Some(c.user_data as i64)
    }

    /// A collider from a part's map: `shape` (`circle`, `box`, `capsule`,
    /// `poly`, `chain`), `size` (a radius, or the whole `(w, h)`),
    /// `round`, `points` (a polygon's or a chain's, local), `at` and
    /// `rot` (its place on the body), `density`, `friction`, `bounce`,
    /// `sensor`, `groups` (`(member, collides_with)`, bit masks),
    /// `one_way` (true: things land on its top and pass up through it; or
    /// the side, a pair, local).
    fn collider(&self, m: &ValueMap, id: i64) -> Result<(ColliderBuilder, Option<Vector>), String> {
        let k = self.k;
        let shape = match m.get("shape") {
            Some(Value::String(s)) => s.to_string(),
            None | Some(Value::Unit) => "circle".to_string(),
            Some(_) => return Err("toad.body: a `shape` is a name".into()),
        };
        let size = m.get("size");
        let half = match size {
            Some(s) => match (num(s), pair(s)) {
                (Some(r), _) => (r, r),
                (_, Some((w, h))) => (w / 2.0, h / 2.0),
                _ => return Err("toad.body: `size` is a number or a pair".into()),
            },
            None => (16.0, 16.0),
        };
        let points = |what: &str| -> Result<Vec<Vector>, String> {
            match m.get("points") {
                Some(Value::List(ps)) => {
                    let pts: Vec<Vector> = ps.iter().filter_map(pair).map(|p| v(p, k)).collect();
                    if pts.len() < 2 || pts.len() != ps.len() {
                        return Err(format!(
                            "toad.body: a {what}'s `points` are two or more pairs"
                        ));
                    }
                    Ok(pts)
                }
                _ => Err(format!("toad.body: a {what} needs `points`")),
            }
        };
        let round = m.get("round").and_then(num).unwrap_or(0.0).max(0.0);
        let b = match shape.as_str() {
            "circle" => ColliderBuilder::ball(half.0 * k),
            "box" if round > 0.0 => {
                let r = round.min(half.0).min(half.1);
                ColliderBuilder::round_cuboid((half.0 - r) * k, (half.1 - r) * k, r * k)
            }
            "box" => ColliderBuilder::cuboid(half.0 * k, half.1 * k),
            // along x, as the renderer draws one: its whole length and thickness
            "capsule" => ColliderBuilder::capsule_x((half.0 - half.1).max(0.0) * k, half.1 * k),
            "poly" => ColliderBuilder::convex_hull(&points("poly")?)
                .ok_or("toad.body: a poly's points make no convex shape")?,
            "chain" => {
                let pts = points("chain")?;
                if flag(m, "closed", false) {
                    let n = pts.len() as u32;
                    let idx = (0..n).map(|i| [i, (i + 1) % n]).collect();
                    ColliderBuilder::polyline(pts, Some(idx))
                } else {
                    ColliderBuilder::polyline(pts, None)
                }
            }
            other => {
                return Err(format!(
                    "toad.body: a shape is a circle, box, capsule, poly or chain, not a {other}"
                ));
            }
        };
        let groups = match m.get("groups").and_then(pair) {
            Some((a, c)) => InteractionGroups::new(
                Group::from_bits_retain(a as u32),
                Group::from_bits_retain(c as u32),
                InteractionTestMode::And,
            ),
            None => InteractionGroups::all(),
        };
        let sensor = flag(m, "sensor", false);
        let mut b = b
            .translation(v(m.get("at").and_then(pair).unwrap_or((0.0, 0.0)), k))
            .rotation(m.get("rot").and_then(num).unwrap_or(0.0))
            .density(m.get("density").and_then(num).unwrap_or(1.0))
            .friction(m.get("friction").and_then(num).unwrap_or(0.5))
            .restitution(m.get("bounce").and_then(num).unwrap_or(0.0))
            .sensor(sensor)
            .collision_groups(groups)
            .active_events(ActiveEvents::COLLISION_EVENTS)
            .user_data(id as u128);
        if sensor {
            // a trigger notices kinematic and fixed bodies too
            b = b.active_collision_types(ActiveCollisionTypes::all());
        }
        let one_way = match m.get("one_way") {
            Some(Value::Boolean(true)) => Some(Vector::new(0.0, 1.0)),
            Some(x) => pair(x).map(|p| Vector::new(p.0, p.1).normalize_or_zero()),
            None => None,
        };
        if one_way.is_some() {
            b = b.active_hooks(ActiveHooks::MODIFY_SOLVER_CONTACTS);
        }
        Ok((b, one_way))
    }

    /// A body: `kind` (`dynamic`, `static`, `kinematic`), `at`, `rot`,
    /// `vel`, `spin`, `damping`, `spin_damping`, `fixed_rot`, `bullet`
    /// (continuous collision: fast and thin), `gravity` (a scale), and its
    /// shape: the collider keys on the body itself, or `parts`, a list of
    /// them. Answers its number.
    pub fn add_body(&mut self, props: &Value) -> Result<i64, String> {
        let m = map_of(props, "a body")?;
        let k = self.k;
        let id = self.next;
        self.next += 1;
        let b = match m.get("kind") {
            None | Some(Value::Unit) => RigidBodyBuilder::dynamic(),
            Some(Value::String(s)) => match s.as_str() {
                "dynamic" => RigidBodyBuilder::dynamic(),
                "static" => RigidBodyBuilder::fixed(),
                "kinematic" => RigidBodyBuilder::kinematic_position_based(),
                other => {
                    return Err(format!(
                        "toad.body: a body is dynamic, static or kinematic, not {other}"
                    ));
                }
            },
            Some(_) => return Err("toad.body: `kind` is a name".into()),
        };
        let at = m.get("at").and_then(pair).unwrap_or((0.0, 0.0));
        let rot = m.get("rot").and_then(num).unwrap_or(0.0);
        let mut b = b
            .translation(v(at, k))
            .rotation(rot)
            .linvel(v(m.get("vel").and_then(pair).unwrap_or((0.0, 0.0)), k))
            .angvel(m.get("spin").and_then(num).unwrap_or(0.0))
            .linear_damping(m.get("damping").and_then(num).unwrap_or(0.0))
            .angular_damping(m.get("spin_damping").and_then(num).unwrap_or(0.0))
            .gravity_scale(m.get("gravity").and_then(num).unwrap_or(1.0))
            .ccd_enabled(flag(m, "bullet", false))
            .user_data(id as u128);
        if flag(m, "fixed_rot", false) {
            b = b.lock_rotations();
        }
        let parts: Vec<ValueMap> = match m.get("parts") {
            Some(Value::List(ps)) => ps
                .iter()
                .map(|p| map_of(p, "a body's part").cloned())
                .collect::<Result<_, _>>()?,
            _ => {
                // the body's own keys are its one part, at its middle
                let mut one = m.clone();
                one.shift_remove("at");
                one.shift_remove("rot");
                vec![one]
            }
        };
        let colliders: Vec<(ColliderBuilder, Option<Vector>)> = parts
            .iter()
            .map(|p| self.collider(p, id))
            .collect::<Result<_, _>>()?;
        let h = self.w.bodies.insert(b);
        for (c, one_way) in colliders {
            let ch = self
                .w
                .colliders
                .insert_with_parent(c, h, &mut self.w.bodies);
            if let Some(n) = one_way {
                self.one_way.insert(ch, n);
            }
        }
        self.bodies.insert(id, h);
        self.prev.insert(id, ([at.0, at.1], rot));
        Ok(id)
    }

    pub fn remove_body(&mut self, id: i64) {
        if let Some(h) = self.bodies.remove(&id) {
            self.prev.remove(&id);
            self.grounded.remove(&id);
            if let Some(b) = self.w.bodies.get(h) {
                for c in b.colliders() {
                    self.one_way.remove(c);
                }
            }
            self.w.remove_body_with_colliders(h, true);
            // its joints went with it
            let joints = &self.w.impulse_joints;
            self.joints.retain(|_, j| joints.get(*j).is_some());
        }
    }

    /// Move a body by hand: `impulse`, `force` (for this step), `torque`,
    /// `spin_impulse`, `vel`, `spin`, `at` and `rot` (a teleport; for a
    /// kinematic body, where it is by the end of the next step).
    pub fn push(&mut self, id: i64, props: &Value) -> Result<(), String> {
        let h = self.handle(id)?;
        let m = map_of(props, "a push")?;
        let k = self.k;
        let b = self.w.bodies.get_mut(h).ok_or("toad: the body is gone")?;
        if let Some(p) = m.get("impulse").and_then(pair) {
            b.apply_impulse(v(p, k), true);
        }
        if let Some(p) = m.get("force").and_then(pair) {
            b.reset_forces(true);
            b.add_force(v(p, k), true);
        }
        if let Some(t) = m.get("torque").and_then(num) {
            b.reset_torques(true);
            b.add_torque(t, true);
        }
        if let Some(t) = m.get("spin_impulse").and_then(num) {
            b.apply_torque_impulse(t, true);
        }
        if let Some(p) = m.get("vel").and_then(pair) {
            b.set_linvel(v(p, k), true);
        }
        if let Some(s) = m.get("spin").and_then(num) {
            b.set_angvel(s, true);
        }
        let at = m.get("at").and_then(pair);
        let rot = m.get("rot").and_then(num);
        if at.is_some() || rot.is_some() {
            let cur = *b.position();
            let t = at.map(|p| v(p, k)).unwrap_or(cur.translation);
            let r = rot.unwrap_or(cur.rotation.angle());
            let pose = Pose::new(t, r);
            if b.is_kinematic() {
                b.set_next_kinematic_position(pose);
            } else {
                b.set_position(pose, true);
                // a teleport is not drawn as a slide
                self.prev.insert(id, ([t.x / k, t.y / k], r));
            }
        }
        Ok(())
    }

    /// A joint between bodies `a` and `b`: `kind` `pin` (they turn about
    /// a point), `weld` (they hold together), `slide` (along `axis`, within
    /// `limits`), `rope` (no further apart than `length`), `spring` (pulled
    /// to `length` by `stiffness`, slowed by `damping`); `at_a`, `at_b`
    /// (the anchors on each, local), `motor` (a pin's or a slide's speed),
    /// `collide` (whether the two still touch). Answers its number.
    pub fn add_joint(&mut self, a: i64, b: i64, props: &Value) -> Result<i64, String> {
        let (ha, hb) = (self.handle(a)?, self.handle(b)?);
        let m = map_of(props, "a joint")?;
        let k = self.k;
        let p1 = v(m.get("at_a").and_then(pair).unwrap_or((0.0, 0.0)), k);
        let p2 = v(m.get("at_b").and_then(pair).unwrap_or((0.0, 0.0)), k);
        let collide = flag(m, "collide", false);
        let motor = m.get("motor").and_then(num);
        let joint: GenericJoint = match m.get("kind") {
            Some(Value::String(s)) => match s.as_str() {
                "pin" => {
                    let mut j = RevoluteJointBuilder::new()
                        .local_anchor1(p1)
                        .local_anchor2(p2)
                        .contacts_enabled(collide);
                    if let Some(sp) = motor {
                        j = j.motor_velocity(sp, 1.0);
                    }
                    if let Some(l) = m.get("limits").and_then(pair) {
                        j = j.limits([l.0, l.1]);
                    }
                    j.into()
                }
                "weld" => FixedJointBuilder::new()
                    .local_anchor1(p1)
                    .local_anchor2(p2)
                    .contacts_enabled(collide)
                    .into(),
                "slide" => {
                    let ax = m.get("axis").and_then(pair).unwrap_or((1.0, 0.0));
                    let ax = Vector::new(ax.0, ax.1).normalize_or_zero();
                    let mut j = PrismaticJointBuilder::new(ax)
                        .local_anchor1(p1)
                        .local_anchor2(p2)
                        .contacts_enabled(collide);
                    if let Some(l) = m.get("limits").and_then(pair) {
                        j = j.limits([l.0 * k, l.1 * k]);
                    }
                    if let Some(sp) = motor {
                        j = j.motor_velocity(sp * k, 1.0);
                    }
                    j.into()
                }
                "rope" => {
                    let len = m
                        .get("length")
                        .and_then(num)
                        .ok_or("toad.joint: a rope needs a `length`")?;
                    RopeJointBuilder::new(len * k)
                        .local_anchor1(p1)
                        .local_anchor2(p2)
                        .contacts_enabled(collide)
                        .into()
                }
                "spring" => {
                    let len = m
                        .get("length")
                        .and_then(num)
                        .ok_or("toad.joint: a spring needs a `length`")?;
                    SpringJointBuilder::new(
                        len * k,
                        m.get("stiffness").and_then(num).unwrap_or(40.0),
                        m.get("damping").and_then(num).unwrap_or(2.0),
                    )
                    .local_anchor1(p1)
                    .local_anchor2(p2)
                    .contacts_enabled(collide)
                    .into()
                }
                other => {
                    return Err(format!(
                        "toad.joint: a joint is a pin, weld, slide, rope or spring, not a {other}"
                    ));
                }
            },
            _ => {
                return Err(
                    "toad.joint: a joint needs a `kind` (pin, weld, slide, rope, spring)".into(),
                );
            }
        };
        let h = self.w.insert_impulse_joint(ha, hb, joint);
        let id = self.next;
        self.next += 1;
        self.joints.insert(id, h);
        Ok(id)
    }

    pub fn remove_joint(&mut self, id: i64) {
        if let Some(h) = self.joints.remove(&id) {
            self.w.remove_impulse_joint(h);
        }
    }

    /// One step of `dt` seconds: every touch that started or stopped, as
    /// `#{ a, b, start, sensor, at, normal }` (a contact's point and its
    /// normal from `a` to `b`, when they touch).
    pub fn simulate(&mut self, dt: f32) -> Value {
        let k = self.k;
        self.prev.clear();
        for (id, h) in &self.bodies {
            if let Some(b) = self.w.bodies.get(*h)
                && !b.is_fixed()
            {
                let t = b.translation();
                self.prev
                    .insert(*id, ([t.x / k, t.y / k], b.rotation().angle()));
            }
        }
        self.w.integration_parameters.dt = dt.max(1e-6);
        let ev = Events::default();
        let hooks = Hooks {
            one_way: &self.one_way,
        };
        self.w.step_with_events(&hooks, &ev);
        let mut raw = std::mem::take(&mut self.pending);
        raw.extend(ev.0.into_inner().unwrap_or_else(|e| e.into_inner()));
        let mut out = Vec::with_capacity(raw.len());
        for e in raw {
            let (Some(a), Some(b)) = (self.id_of_collider(e.a), self.id_of_collider(e.b)) else {
                continue;
            };
            let mut entries = vec![
                ("a", Value::Integer(a)),
                ("b", Value::Integer(b)),
                ("start", Value::Boolean(e.start)),
                ("sensor", Value::Boolean(e.sensor)),
            ];
            if let Some((at, n)) = e.point {
                entries.push(("at", tup(&[at[0] / k, at[1] / k])));
                entries.push(("normal", tup(&n)));
            }
            out.push(map(entries));
        }
        Value::List(Arc::new(out))
    }

    /// Build what the first step would (the broad phase, the contacts),
    /// so a large world's first step costs no more than the rest; the
    /// touches found are told with that step.
    pub fn warm(&mut self) {
        // a step of no time: the pipeline's own broad and narrow phase built
        // and nothing moved (rapier keeps a zero-length step still; its
        // collision pipeline alone would leave pairs the solver refuses)
        let ev = Events::default();
        let hooks = Hooks {
            one_way: &self.one_way,
        };
        let dt = self.w.integration_parameters.dt;
        self.w.integration_parameters.dt = 0.0;
        self.w.step_with_events(&hooks, &ev);
        self.w.integration_parameters.dt = dt;
        self.pending
            .extend(ev.0.into_inner().unwrap_or_else(|e| e.into_inner()));
    }

    /// Walk a kinematic body by `delta` (world units) as a character does:
    /// sliding along walls, up `slope` (degrees, 45) and onto steps up to
    /// `step` high, snapped down to the ground within `snap`, up through
    /// one-way platforms and onto them. It gets there by the end of the
    /// next step. Answers `#{ moved, grounded, sliding }`.
    pub fn walk(
        &mut self,
        id: i64,
        delta: (f32, f32),
        opts: &Value,
        dt: f32,
    ) -> Result<Value, String> {
        let h = self.handle(id)?;
        let k = self.k;
        let empty = ValueMap::default();
        let m = match opts {
            Value::Unit => &empty,
            o => map_of(o, "the walk's options")?,
        };
        let b = self.w.bodies.get(h).ok_or("toad: the body is gone")?;
        if !b.is_kinematic() {
            return Err("toad.walk: a walking body is kinematic (`kind: \"kinematic\"`)".into());
        }
        let ch = *b
            .colliders()
            .first()
            .ok_or("toad.walk: the body has no shape")?;
        let co = self
            .w
            .colliders
            .get(ch)
            .ok_or("toad: the body's shape is gone")?;
        let shape = co.shared_shape().clone();
        let pos = *co.position();
        let bottom = co.compute_aabb().mins.y;
        let n = |key: &str, d: f32| m.get(key).and_then(num).unwrap_or(d);
        let ctl = KinematicCharacterController {
            up: Vector::new(0.0, 1.0),
            offset: CharacterLength::Absolute(0.5 * k),
            slide: true,
            autostep: match m.get("step").and_then(num) {
                Some(hgt) if hgt > 0.0 => Some(CharacterAutostep {
                    max_height: CharacterLength::Absolute(hgt * k),
                    min_width: CharacterLength::Absolute(hgt * 0.5 * k),
                    include_dynamic_bodies: false,
                }),
                _ => None,
            },
            max_slope_climb_angle: n("slope", 45.0).to_radians(),
            min_slope_slide_angle: (n("slope", 45.0) + 5.0).to_radians(),
            snap_to_ground: Some(CharacterLength::Absolute(n("snap", 4.0) * k)),
            ..Default::default()
        };
        let up = delta.1 > 0.0;
        let one_way = &self.one_way;
        let colliders = &self.w.colliders;
        // a one-way platform stops a walker only from above: not while it
        // rises, nor while any of it is below the platform's top
        let pass = |c: ColliderHandle, _co: &Collider| -> bool {
            match one_way.get(&c) {
                None => true,
                Some(_) => {
                    let top = colliders
                        .get(c)
                        .map(|p| p.compute_aabb().maxs.y)
                        .unwrap_or(f32::MIN);
                    !up && bottom >= top - 0.02
                }
            }
        };
        let filter = QueryFilter::default()
            .exclude_sensors()
            .exclude_rigid_body(h)
            .predicate(&pass);
        let q = self.w.query_pipeline_with_filter(filter);
        let mv = ctl.move_shape(dt.max(1e-6), &q, &*shape, &pos, v(delta, k), |_| {});
        let b = self.w.bodies.get_mut(h).ok_or("toad: the body is gone")?;
        let to = b.translation() + mv.translation;
        b.set_next_kinematic_translation(to);
        self.grounded.insert(id, mv.grounded);
        Ok(map(vec![
            ("moved", tup(&[mv.translation.x / k, mv.translation.y / k])),
            ("grounded", Value::Boolean(mv.grounded)),
            ("sliding", Value::Boolean(mv.is_sliding_down_slope)),
        ]))
    }

    /// A body's pose and motion now and before the last step:
    /// `#{ at, rot, vel, spin, was_at, was_rot }`, or `()`.
    pub fn pose(&self, id: i64) -> Value {
        let k = self.k;
        let Some(b) = self.bodies.get(&id).and_then(|h| self.w.bodies.get(*h)) else {
            return Value::Unit;
        };
        let t = b.translation();
        let at = [t.x / k, t.y / k];
        let rot = b.rotation().angle();
        let (was_at, was_rot) = self.prev.get(&id).copied().unwrap_or((at, rot));
        let lv = b.linvel();
        let grounded = self.grounded.get(&id).copied().unwrap_or(false);
        map(vec![
            ("grounded", Value::Boolean(grounded)),
            ("at", tup(&at)),
            ("rot", Value::Float(rot as f64)),
            ("vel", tup(&[lv.x / k, lv.y / k])),
            ("spin", Value::Float(b.angvel() as f64)),
            ("was_at", tup(&was_at)),
            ("was_rot", Value::Float(was_rot as f64)),
        ])
    }

    /// The first body a ray from `from` to `to` meets:
    /// `#{ body, at, normal, t }` (`t` from 0 at `from` to 1 at `to`), or
    /// `()`. `opts`: `sensors` (whether they stop it), `skip` (a body it
    /// passes through).
    pub fn ray(&self, from: (f32, f32), to: (f32, f32), opts: &Value) -> Result<Value, String> {
        let k = self.k;
        let empty = ValueMap::default();
        let m = match opts {
            Value::Unit => &empty,
            o => map_of(o, "the ray's options")?,
        };
        let d = (to.0 - from.0, to.1 - from.1);
        let len = (d.0 * d.0 + d.1 * d.1).sqrt();
        if len <= 0.0 {
            return Ok(Value::Unit);
        }
        let ray = Ray::new(v(from, k), Vector::new(d.0 / len, d.1 / len));
        let mut filter = QueryFilter::default();
        if !flag(m, "sensors", false) {
            filter = filter.exclude_sensors();
        }
        if let Some(Value::Integer(s)) = m.get("skip")
            && let Some(h) = self.bodies.get(s)
        {
            filter = filter.exclude_rigid_body(*h);
        }
        let Some((h, hit)) = self.w.cast_ray_and_get_normal(&ray, len * k, true, filter) else {
            return Ok(Value::Unit);
        };
        let t = hit.time_of_impact / k;
        let Some(id) = self.id_of_collider(h) else {
            return Ok(Value::Unit);
        };
        Ok(map(vec![
            ("body", Value::Integer(id)),
            ("at", tup(&[from.0 + d.0 / len * t, from.1 + d.1 / len * t])),
            ("normal", tup(&[hit.normal.x, hit.normal.y])),
            ("t", Value::Float((t / len) as f64)),
        ]))
    }

    /// The bodies within `radius` of `at`, by number, each once, in order.
    pub fn overlap(&self, at: (f32, f32), radius: f32, opts: &Value) -> Result<Value, String> {
        let k = self.k;
        let empty = ValueMap::default();
        let m = match opts {
            Value::Unit => &empty,
            o => map_of(o, "the query's options")?,
        };
        let mut filter = QueryFilter::default();
        if !flag(m, "sensors", true) {
            filter = filter.exclude_sensors();
        }
        let ball = Ball::new((radius * k).max(1e-6));
        let q = self.w.query_pipeline_with_filter(filter);
        let mut ids: Vec<i64> = q
            .intersect_shape(Pose::from_translation(v(at, k)), &ball)
            .map(|(_, c)| c.user_data as i64)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        Ok(Value::List(Arc::new(
            ids.into_iter().map(Value::Integer).collect(),
        )))
    }

    /// The solid colliders as the GPU tests them, in world units: 8
    /// floats each (centre, turn, kind, half size, rounding, room): kind
    /// 0 a circle (radius in the rounding), 1 a box (rounded), 2 a capsule
    /// along x (a chain is one per segment). A polygon is its bounds.
    pub fn gpu_shapes(&self) -> Vec<f32> {
        let k = self.k;
        let mut out = Vec::new();
        for (_, co) in self.w.colliders.iter() {
            if co.is_sensor() {
                continue;
            }
            let pose = co.position();
            let rot = pose.rotation.angle();
            let c = pose.translation;
            let sh = co.shape();
            let mut push = |cx: f32, cy: f32, r: f32, kind: f32, hx: f32, hy: f32, round: f32| {
                out.extend_from_slice(&[cx / k, cy / k, r, kind, hx / k, hy / k, round / k, 0.0]);
            };
            if let Some(b) = sh.as_ball() {
                push(c.x, c.y, rot, 0.0, b.radius, b.radius, b.radius);
            } else if let Some(b) = sh.as_cuboid() {
                push(c.x, c.y, rot, 1.0, b.half_extents.x, b.half_extents.y, 0.0);
            } else if let Some(b) = sh.as_round_cuboid() {
                let r = b.border_radius;
                let h = b.inner_shape.half_extents;
                push(c.x, c.y, rot, 1.0, h.x + r, h.y + r, r);
            } else if let Some(cap) = sh.as_capsule() {
                let (a, b) = (
                    pose.transform_point(cap.segment.a),
                    pose.transform_point(cap.segment.b),
                );
                let d = b - a;
                let m = (a + b) * 0.5;
                push(
                    m.x,
                    m.y,
                    d.y.atan2(d.x),
                    2.0,
                    d.length() * 0.5,
                    cap.radius,
                    cap.radius,
                );
            } else if let Some(pl) = sh.as_polyline() {
                for seg in pl.segments() {
                    let (a, b) = (pose.transform_point(seg.a), pose.transform_point(seg.b));
                    let d = b - a;
                    let m = (a + b) * 0.5;
                    push(m.x, m.y, d.y.atan2(d.x), 2.0, d.length() * 0.5, 0.0, 0.0);
                }
            } else {
                let bb = co.compute_aabb();
                let m = (bb.mins + bb.maxs) * 0.5;
                let h = (bb.maxs - bb.mins) * 0.5;
                push(m.x, m.y, 0.0, 1.0, h.x, h.y, 0.0);
            }
        }
        out
    }

    /// The number of bodies and joints.
    pub fn counts(&self) -> (usize, usize) {
        (self.bodies.len(), self.joints.len())
    }
}

/// One-way platforms: a contact with one stands only when it pushes the
/// other body out of the platform's open side.
struct Hooks<'a> {
    one_way: &'a HashMap<ColliderHandle, Vector>,
}

impl PhysicsHooks for Hooks<'_> {
    fn modify_solver_contacts(&self, context: &mut ContactModificationContext) {
        let angle = std::f32::consts::FRAC_PI_4;
        if let Some(n) = self.one_way.get(&context.collider1) {
            context.update_as_oneway_platform(*n, angle);
        } else if let Some(n) = self.one_way.get(&context.collider2) {
            context.update_as_oneway_platform(-*n, angle);
        }
    }
}

/// A touch, as the solver told it.
struct Touch {
    a: ColliderHandle,
    b: ColliderHandle,
    start: bool,
    sensor: bool,
    point: Option<([f32; 2], [f32; 2])>,
}

#[derive(Default)]
struct Events(Mutex<Vec<Touch>>);

impl EventHandler for Events {
    fn handle_collision_event(
        &self,
        _bodies: &RigidBodySet,
        colliders: &ColliderSet,
        event: CollisionEvent,
        pair: Option<&ContactPair>,
    ) {
        let (a, b, start, flags) = match event {
            CollisionEvent::Started(a, b, f) => (a, b, true, f),
            CollisionEvent::Stopped(a, b, f) => (a, b, false, f),
        };
        let point = pair.and_then(|p| {
            let (m, c) = p.find_deepest_contact()?;
            let co = colliders.get(p.collider1)?;
            let at = co.position().transform_point(c.local_p1);
            let n = m.data.normal;
            // from `a` to `b` as the event names them
            let n = if p.collider1 == a { n } else { -n };
            Some(([at.x, at.y], [n.x, n.y]))
        });
        if let Ok(mut g) = self.0.lock() {
            g.push(Touch {
                a,
                b,
                start,
                sensor: flags.contains(CollisionEventFlags::SENSOR),
                point,
            });
        }
    }

    fn handle_contact_force_event(
        &self,
        _dt: Real,
        _bodies: &RigidBodySet,
        _colliders: &ColliderSet,
        _pair: &ContactPair,
        _total: Real,
    ) {
    }

    fn handle_soft_body_tear_event(&self, _soft: &SoftBodySet, _event: &SoftBodyTearEvent) {}
}

// ── swarms ──────────────────────────────────────────────────────────────

/// Where a swarm's position, velocity and an optional per-entity number
/// are: column indices.
pub struct Cols {
    pos: usize,
    vel: Option<usize>,
    radius: Option<usize>,
    mark: Option<usize>,
}

fn f_slot(p: &Program, name: &str, ty: Ty, what: &str) -> Result<usize, String> {
    p.fields
        .get(name)
        .filter(|s| s.ty == ty)
        .map(|s| s.at)
        .ok_or_else(|| {
            format!(
                "{what}: the swarm has no {} `{name}`",
                if ty == Ty::V2 { "pair" } else { "number" }
            )
        })
}

fn opt_field(
    p: &Program,
    m: &ValueMap,
    key: &str,
    ty: Ty,
    what: &str,
) -> Result<Option<usize>, String> {
    match m.get(key) {
        Some(Value::String(f)) => f_slot(p, f, ty, what).map(Some),
        _ => Ok(None),
    }
}

impl Cols {
    fn of(p: &Program, m: &ValueMap, what: &str, mark_key: &str) -> Result<Cols, String> {
        let pos = f_slot(p, "pos", Ty::V2, what)?;
        let vel = p.fields.get("vel").filter(|s| s.ty == Ty::V2).map(|s| s.at);
        Ok(Cols {
            pos,
            vel,
            radius: opt_field(p, m, "radius", Ty::F, what)?,
            mark: opt_field(p, m, mark_key, Ty::F, what)?,
        })
    }
}

/// One entity, as the grid works on it.
#[derive(Clone, Copy, Default)]
struct Ent {
    p: [f32; 2],
    v: [f32; 2],
    r: f32,
    hit: bool,
    /// The collider it hit last (an index into the grid's list), and the
    /// push it gave that collider's body (world units · mass).
    body: u32,
    push: [f32; 2],
}

const NONE: u32 = u32::MAX;

fn read_ents(cols: &Columns, c: &Cols, r: f32) -> Vec<Ent> {
    let cap = cols.cap;
    let d = &cols.data;
    (0..cols.n)
        .map(|i| Ent {
            p: [d[c.pos * cap + i], d[(c.pos + 1) * cap + i]],
            v: c.vel
                .map(|vc| [d[vc * cap + i], d[(vc + 1) * cap + i]])
                .unwrap_or([0.0, 0.0]),
            r: c.radius.map(|rc| d[rc * cap + i]).unwrap_or(r),
            hit: false,
            body: NONE,
            push: [0.0, 0.0],
        })
        .collect()
}

fn write_ents(cols: &mut Columns, c: &Cols, ents: &[Ent]) {
    let cap = cols.cap;
    for (i, e) in ents.iter().enumerate() {
        cols.data[c.pos * cap + i] = e.p[0];
        cols.data[(c.pos + 1) * cap + i] = e.p[1];
        if let Some(vc) = c.vel {
            cols.data[vc * cap + i] = e.v[0];
            cols.data[(vc + 1) * cap + i] = e.v[1];
        }
        if let Some(mc) = c.mark {
            cols.data[mc * cap + i] = if e.hit { 1.0 } else { 0.0 };
        }
    }
}

/// A uniform grid over the plane: cell → the items in it.
struct Grid {
    cell: f32,
    cells: HashMap<(i32, i32), Vec<u32>>,
}

impl Grid {
    fn new(cell: f32) -> Grid {
        Grid {
            cell: cell.max(1e-3),
            cells: HashMap::new(),
        }
    }
    fn key(&self, x: f32, y: f32) -> (i32, i32) {
        (
            (x / self.cell).floor() as i32,
            (y / self.cell).floor() as i32,
        )
    }
    fn insert_box(&mut self, lo: [f32; 2], hi: [f32; 2], item: u32) {
        let (a, b) = (self.key(lo[0], lo[1]), self.key(hi[0], hi[1]));
        // a collider larger than the grid's reason is put in once per cell
        // it covers, up to a bound: past it the plane is that big anyway
        let span = ((b.0 - a.0 + 1) as i64) * ((b.1 - a.1 + 1) as i64);
        if span > 1 << 20 {
            return;
        }
        for x in a.0..=b.0 {
            for y in a.1..=b.1 {
                self.cells.entry((x, y)).or_default().push(item);
            }
        }
    }
    fn at(&self, x: f32, y: f32) -> &[u32] {
        self.cells
            .get(&self.key(x, y))
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }
}

/// Run `f` over the entities in chunks, on every core where there are
/// cores; the same result either way.
fn each_chunk(ents: &mut [Ent], f: impl Fn(&mut [Ent]) + Sync + Send) {
    #[cfg(feature = "native")]
    crate::parallel::install(|| ents.par_chunks_mut(1024).for_each(f));
    #[cfg(not(feature = "native"))]
    ents.chunks_mut(1024).for_each(f);
}

/// A play swarm against the world's bodies: each entity (its `pos`, a
/// circle of `radius`, a number or a field) pushed out of what it
/// touches; its `vel` (when it has one) bounced by `bounce` (0 to 1) and
/// slowed along the surface by `friction` (a bounce slower than `rest`,
/// 20 world units a second, is none: what lands settles). `die`: those
/// that touch die;
/// `mark`: a field set to 1 for those that touched this step, 0 for the
/// rest; `push`: each entity's mass, pushing dynamic bodies it hits;
/// `sensors`: whether sensors count (as touches, never as walls).
/// Per body: (id, entities that touched it); then the dead flags.
pub type Collisions = (Vec<(i64, i64)>, Vec<bool>);

/// Answers `(per body: (id, entities that touched it), dead flags)`.
pub fn collide(
    w: &mut World,
    p: &Program,
    cols: &mut Columns,
    opts: &Value,
) -> Result<Collisions, String> {
    let m = map_of(opts, "the collision's options")?;
    let what = "toad.collide";
    let c = Cols::of(p, m, what, "mark")?;
    let r0 = m.get("radius").and_then(num).unwrap_or(4.0);
    let bounce = m.get("bounce").and_then(num).unwrap_or(0.5).clamp(0.0, 1.0);
    let friction = m
        .get("friction")
        .and_then(num)
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    let die = flag(m, "die", false);
    let push = m.get("push").and_then(num).unwrap_or(0.0);
    let rest = m.get("rest").and_then(num).unwrap_or(20.0).max(0.0);
    let sensors = flag(m, "sensors", false);
    let k = w.k;
    let mut ents = read_ents(cols, &c, r0);
    let rmax = ents.iter().fold(r0, |a, e| a.max(e.r));
    // the colliders, each in the cells its box (grown by the largest
    // radius) covers
    let mut list: Vec<(ColliderHandle, i64, bool)> = Vec::new();
    let mut grid = Grid::new((rmax * 4.0).max(48.0));
    for (h, co) in w.w.colliders.iter() {
        if co.is_sensor() && !sensors {
            continue;
        }
        let bb = co.compute_aabb();
        let lo = [bb.mins.x / k - rmax, bb.mins.y / k - rmax];
        let hi = [bb.maxs.x / k + rmax, bb.maxs.y / k + rmax];
        grid.insert_box(lo, hi, list.len() as u32);
        list.push((h, co.user_data as i64, co.is_sensor()));
    }
    let colliders = &w.w.colliders;
    each_chunk(&mut ents, |chunk| {
        for e in chunk.iter_mut() {
            for &ci in grid.at(e.p[0], e.p[1]) {
                let (h, _, sensor) = list[ci as usize];
                let Some(co) = colliders.get(h) else { continue };
                let pt = Vector::new(e.p[0] * k, e.p[1] * k);
                let proj = co.shape().project_point(co.position(), pt, false);
                let to = [(e.p[0] - proj.point.x / k), (e.p[1] - proj.point.y / k)];
                let dist = (to[0] * to[0] + to[1] * to[1]).sqrt();
                // resting on it (pushed out to its radius last step) is touching
                if !proj.is_inside && dist > e.r + 1e-3 {
                    continue;
                }
                e.hit = true;
                e.body = ci;
                if sensor {
                    continue;
                }
                // the way out: from the surface (the boundary's nearest point,
                // inside or out) toward the entity, or (inside) the other way
                let mut n = if dist > 1e-6 {
                    [to[0] / dist, to[1] / dist]
                } else {
                    [0.0, 1.0]
                };
                if proj.is_inside {
                    n = [-n[0], -n[1]];
                }
                let depth = if proj.is_inside {
                    e.r + dist
                } else {
                    e.r - dist
                };
                e.p = [e.p[0] + n[0] * depth, e.p[1] + n[1] * depth];
                let vn = e.v[0] * n[0] + e.v[1] * n[1];
                if vn < 0.0 {
                    let vt = [e.v[0] - n[0] * vn, e.v[1] - n[1] * vn];
                    // a bounce slower than `rest` is none: what lands settles
                    let back = if -vn * bounce < rest {
                        0.0
                    } else {
                        -vn * bounce
                    };
                    e.v = [
                        vt[0] * (1.0 - friction) + n[0] * back,
                        vt[1] * (1.0 - friction) + n[1] * back,
                    ];
                    e.push = [
                        e.push[0] + n[0] * vn * push * (1.0 + bounce),
                        e.push[1] + n[1] * vn * push * (1.0 + bounce),
                    ];
                }
            }
        }
    });
    // the bodies' tallies and pushes, put together in entity order
    let mut tally: Vec<i64> = vec![0; list.len()];
    let mut pushes: Vec<[f32; 2]> = vec![[0.0, 0.0]; list.len()];
    for e in &ents {
        if e.body != NONE {
            tally[e.body as usize] += 1;
            let p = &mut pushes[e.body as usize];
            p[0] += e.push[0];
            p[1] += e.push[1];
        }
    }
    let mut per_body: Vec<(i64, i64)> = Vec::new();
    for (i, (h, id, _)) in list.iter().enumerate() {
        if tally[i] == 0 {
            continue;
        }
        match per_body.iter_mut().find(|(b, _)| b == id) {
            Some(x) => x.1 += tally[i],
            None => per_body.push((*id, tally[i])),
        }
        if push > 0.0
            && let Some(parent) = colliders.get(*h).and_then(|co| co.parent())
            && let Some(b) = w.w.bodies.get_mut(parent)
            && b.is_dynamic()
        {
            b.apply_impulse(Vector::new(pushes[i][0] * k, pushes[i][1] * k), true);
        }
    }
    per_body.sort_unstable();
    write_ents(cols, &c, &ents);
    let dead = if die {
        ents.iter().map(|e| e.hit).collect()
    } else {
        Vec::new()
    };
    Ok((per_body, dead))
}

/// Two play swarms meeting: an entity of `a` and one of `b` meet when
/// their circles (`radius_a`, `radius_b`: numbers or fields) touch.
/// `kill`: `"a"`, `"b"`, `"both"` (each of `b` taken by one of `a` at
/// most, the first in `a`'s order) or `"none"`; `mark_a`, `mark_b`:
/// fields set to 1 for those that met this step. Answers
/// `(meetings, a's dead flags, b's dead flags)`.
pub fn meet(
    pa: &Program,
    a: &mut Columns,
    pb: &Program,
    b: &mut Columns,
    opts: &Value,
) -> Result<(i64, Vec<bool>, Vec<bool>), String> {
    let m = map_of(opts, "the meeting's options")?;
    let what = "toad.meet";
    let mut ma = ValueMap::default();
    let mut mb = ValueMap::default();
    for (src, dst) in [("radius_a", "radius"), ("mark_a", "mark")] {
        if let Some(x) = m.get(src) {
            ma.insert(dst.into(), x.clone());
        }
    }
    for (src, dst) in [("radius_b", "radius"), ("mark_b", "mark")] {
        if let Some(x) = m.get(src) {
            mb.insert(dst.into(), x.clone());
        }
    }
    let ca = Cols::of(pa, &ma, what, "mark")?;
    let cb = Cols::of(pb, &mb, what, "mark")?;
    let ra = m.get("radius_a").and_then(num).unwrap_or(4.0);
    let rb = m.get("radius_b").and_then(num).unwrap_or(4.0);
    let kill = match m.get("kill") {
        Some(Value::String(s)) => s.to_string(),
        _ => "none".to_string(),
    };
    if !["a", "b", "both", "none"].contains(&kill.as_str()) {
        return Err(format!("{what}: `kill` is a, b, both or none, not {kill}"));
    }
    let mut ea = read_ents(a, &ca, ra);
    let mut eb = read_ents(b, &cb, rb);
    let rmax = ea.iter().fold(ra, |x, e| x.max(e.r)) + eb.iter().fold(rb, |x, e| x.max(e.r));
    let mut grid = Grid::new(rmax);
    for (i, e) in eb.iter().enumerate() {
        grid.insert_box(e.p, e.p, i as u32);
    }
    let exclusive = kill == "both" || kill == "b";
    // candidates in parallel: each of a's nearest touching b
    let near: Vec<u32> = {
        let find = |e: &Ent| -> u32 {
            let (cx, cy) = grid.key(e.p[0], e.p[1]);
            let mut best = (f32::MAX, NONE);
            for dx in -1..=1 {
                for dy in -1..=1 {
                    let Some(items) = grid.cells.get(&(cx + dx, cy + dy)) else {
                        continue;
                    };
                    for &j in items {
                        let o = &eb[j as usize];
                        let d = (o.p[0] - e.p[0]).powi(2) + (o.p[1] - e.p[1]).powi(2);
                        let reach = e.r + o.r;
                        if d <= reach * reach && (d, j) < best {
                            best = (d, j);
                        }
                    }
                }
            }
            best.1
        };
        #[cfg(feature = "native")]
        {
            crate::parallel::install(|| ea.par_iter().map(find).collect())
        }
        #[cfg(not(feature = "native"))]
        {
            ea.iter().map(find).collect()
        }
    };
    // then in a's order: who takes whom
    let mut taken = vec![false; eb.len()];
    let mut met = 0i64;
    for (i, &j) in near.iter().enumerate() {
        if j == NONE {
            continue;
        }
        if exclusive && taken[j as usize] {
            continue;
        }
        taken[j as usize] = true;
        ea[i].hit = true;
        eb[j as usize].hit = true;
        met += 1;
    }
    // a non-exclusive meeting marks every b that any a touched
    write_ents(a, &ca, &ea);
    write_ents(b, &cb, &eb);
    let dead_a = if kill == "a" || kill == "both" {
        ea.iter().map(|e| e.hit).collect()
    } else {
        Vec::new()
    };
    let dead_b = if kill == "b" || kill == "both" {
        eb.iter().map(|e| e.hit).collect()
    } else {
        Vec::new()
    };
    Ok((met, dead_a, dead_b))
}
