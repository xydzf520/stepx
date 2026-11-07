//! Fixed search, a reserved capsule slot, then right-aligned actions.
use super::{widgets::El, *};

const SEARCH_WIDTH: f32 = 320.;
const CAPSULE_WIDTH: f32 = 232.;
const GAP: f32 = 12.;

struct HeaderGeometry {
    compact: bool,
    search_width: f32,
    capsule_width: f32,
    actions_x: f32,
    controls_x: f32,
}

fn geometry(width: f32, full_actions: f32, compact_actions: f32, controls: f32) -> HeaderGeometry {
    // Reserve the capsule's maximum width even when there is no live activity.
    // Arrival, dismissal and carousel rotation must never change the action mode.
    let compact = width < SEARCH_WIDTH + CAPSULE_WIDTH + GAP * 3. + full_actions + controls;
    let actions_width = if compact {
        compact_actions
    } else {
        full_actions
    };
    let controls_x = width - controls;
    let actions_x = controls_x - GAP - actions_width;
    // Only exceptionally small windows may shrink the search; content changes cannot.
    let search_width = SEARCH_WIDTH.min((actions_x - GAP).max(0.));
    let capsule_width = (actions_x - search_width - GAP * 2.).clamp(0., CAPSULE_WIDTH);
    HeaderGeometry {
        compact,
        search_width,
        capsule_width: if capsule_width >= 100. {
            capsule_width
        } else {
            0.
        },
        actions_x,
        controls_x,
    }
}

pub(super) struct HeaderBar {
    children: [El; 5],
    compact: bool,
    capsule_visible: bool,
    positions: [Vector2F; 5],
    size: Option<Vector2F>,
    origin: Option<Point>,
    search_width: std::rc::Rc<Cell<f32>>,
}

impl HeaderBar {
    pub fn new(
        capsule: El,
        search: El,
        actions: El,
        compact: El,
        controls: El,
        search_width: std::rc::Rc<Cell<f32>>,
    ) -> Self {
        Self {
            children: [capsule, search, actions, compact, controls],
            compact: false,
            capsule_visible: false,
            positions: [Vector2F::zero(); 5],
            size: None,
            origin: None,
            search_width,
        }
    }
    fn active(&self) -> impl Iterator<Item = usize> {
        [
            Some(1),
            self.capsule_visible.then_some(0),
            Some(if self.compact { 3 } else { 2 }),
            Some(4),
        ]
        .into_iter()
        .flatten()
    }
}

impl Element for HeaderBar {
    fn layout(&mut self, c: SizeConstraint, ctx: &mut LayoutContext, app: &AppContext) -> Vector2F {
        let width = c.max.x();
        let loose = SizeConstraint {
            min: Vector2F::zero(),
            max: vec2f(width, 32.),
        };
        let controls = self.children[4].layout(loose, ctx, app);
        let full_actions = self.children[2].layout(loose, ctx, app);
        let compact_actions = self.children[3].layout(loose, ctx, app);
        let g = geometry(width, full_actions.x(), compact_actions.x(), controls.x());
        self.compact = g.compact;
        self.capsule_visible = g.capsule_width > 0.;
        let capsule = if self.capsule_visible {
            self.children[0].layout(
                SizeConstraint {
                    max: vec2f(g.capsule_width, 32.),
                    ..loose
                },
                ctx,
                app,
            )
        } else {
            Vector2F::zero()
        };
        let index = if self.compact { 3 } else { 2 };
        let actions = if self.compact {
            compact_actions
        } else {
            full_actions
        };
        let search = self.children[1].layout(
            SizeConstraint {
                min: vec2f(g.search_width, 32.),
                max: vec2f(g.search_width, 32.),
            },
            ctx,
            app,
        );
        self.search_width.set(search.x());
        self.positions[0] = vec2f(g.search_width + GAP, (32. - capsule.y()) * 0.5);
        self.positions[1] = vec2f(0., (32. - search.y()) * 0.5);
        self.positions[index] = vec2f(g.actions_x, (32. - actions.y()) * 0.5);
        self.positions[4] = vec2f(g.controls_x, (32. - controls.y()) * 0.5);
        self.size = Some(vec2f(width, 32.));
        self.size.unwrap()
    }
    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        for i in self.active() {
            self.children[i].after_layout(ctx, app);
        }
    }
    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        for i in self.active() {
            self.children[i].paint(origin + self.positions[i], ctx, app);
        }
    }
    fn dispatch_event(
        &mut self,
        e: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        // Siblings do not overlap; deliver motion to each so old hover state clears.
        let mut handled = false;
        for i in self.active() {
            handled |= self.children[i].dispatch_event(e, ctx, app);
        }
        handled
    }
    fn size(&self) -> Option<Vector2F> {
        self.size
    }
    fn origin(&self) -> Option<Point> {
        self.origin
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_stays_fixed_and_capsule_precedes_right_actions() {
        for width in [720., 980., 1500.] {
            let g = geometry(width, 340., 100., 100.);
            assert_eq!(g.search_width, SEARCH_WIDTH);
            assert!(g.search_width + GAP + g.capsule_width <= g.actions_x - GAP);
            assert_eq!(g.controls_x, width - 100.);
        }
    }

    #[test]
    fn full_actions_require_room_for_capsule_even_when_it_is_absent() {
        let required = SEARCH_WIDTH + CAPSULE_WIDTH + GAP * 3. + 340. + 100.;
        assert!(geometry(required - 1., 340., 100., 100.).compact);
        let g = geometry(required, 340., 100., 100.);
        assert!(!g.compact);
        assert_eq!(g.capsule_width, CAPSULE_WIDTH);
    }

    #[test]
    fn small_windows_fold_capsule_before_shrinking_search() {
        let g = geometry(580., 340., 100., 100.);
        assert!(g.compact);
        assert_eq!(g.search_width, SEARCH_WIDTH);
        assert_eq!(g.capsule_width, 0.);
        let tiny = geometry(500., 340., 100., 100.);
        assert!(tiny.search_width >= 0.);
        assert!(tiny.search_width <= tiny.actions_x - GAP);
    }
}
