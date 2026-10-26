//! A decorative matrix is one layout/hit region, not 391 interactive widgets.
use facloud_ui::{
    color::ColorU,
    elements::*,
    event::{DispatchedEvent, Event},
};
use pathfinder_geometry::{
    rect::RectF,
    vector::{vec2f, Vector2F},
};
use std::sync::Arc;

pub(super) struct LedGrid {
    frame: Vec<u8>,
    dot: f32,
    gap: f32,
    size: Option<Vector2F>,
    origin: Option<Point>,
    on_pixel: Option<Arc<dyn Fn(usize, usize) -> super::Action + Send + Sync>>,
}

impl LedGrid {
    pub fn new(frame: &[u8], dot: f32, gap: f32) -> Self {
        Self {
            frame: frame.to_vec(),
            dot,
            gap,
            size: None,
            origin: None,
            on_pixel: None,
        }
    }

    pub fn on_pixel_click(
        mut self,
        f: impl Fn(usize, usize) -> super::Action + Send + Sync + 'static,
    ) -> Self {
        self.on_pixel = Some(Arc::new(f));
        self
    }

    fn natural_size(&self) -> Vector2F {
        vec2f(
            17. * self.dot + 16. * self.gap,
            23. * self.dot + 22. * self.gap,
        )
    }
}

impl Element for LedGrid {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        _: &mut LayoutContext,
        _: &AppContext,
    ) -> Vector2F {
        let size = self.natural_size().max(constraint.min).min(constraint.max);
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, _: &mut AfterLayoutContext, _: &AppContext) {}

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, _: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        // Keep the entire thumbnail in its parent's hit layer so selecting a
        // preset still works, including the gaps between LEDs. Individual dots
        // have no actions and must not each rebuild the scene's spatial index.
        ctx.scene
            .draw_rect_with_hit_recording(RectF::new(origin, self.size.unwrap()));
        let radius = CornerRadius::with_all(Radius::Pixels((self.dot * 0.25).min(self.dot / 2.)));
        for y in 0..23 {
            for x in 0..17 {
                if stepx_companion::device::is_backscreen_corner(x, y) {
                    continue;
                }
                let brightness = self.frame.get(y * 17 + x).copied().unwrap_or(0);
                let color = match brightness {
                    191.. => ColorU::new(255, 255, 255, 255),
                    91..=190 => ColorU::new(161, 161, 170, 255),
                    _ => ColorU::new(39, 39, 46, 255),
                };
                let offset = vec2f(x as f32, y as f32) * (self.dot + self.gap);
                ctx.scene
                    .draw_rect_without_hit_recording(RectF::new(
                        origin + offset,
                        vec2f(self.dot, self.dot),
                    ))
                    .with_background(Fill::Solid(color))
                    .with_corner_radius(radius);
            }
        }
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        _: &AppContext,
    ) -> bool {
        let Some(cb) = &self.on_pixel else {
            return false;
        };
        let (Some(origin), Some(size)) = (self.origin, self.size) else {
            return false;
        };
        match event.raw_event() {
            Event::RightMouseDown { position, .. } => {
                if position.x() < origin.x()
                    || position.y() < origin.y()
                    || position.x() > origin.x() + size.x()
                    || position.y() > origin.y() + size.y()
                    || ctx.is_covered(Point::from_vec2f(*position, origin.z_index()))
                {
                    return false;
                }
                ctx.dispatch_typed_action(super::Action::ContextOpen(
                    super::context_menu::Target::BackScreenCanvas,
                    *position,
                ));
                return true;
            }
            Event::LeftMouseDown { position, .. } => {
                if position.x() < origin.x()
                    || position.y() < origin.y()
                    || position.x() > origin.x() + size.x()
                    || position.y() > origin.y() + size.y()
                    || ctx.is_covered(Point::from_vec2f(*position, origin.z_index()))
                {
                    return false;
                }
                let pitch = (self.dot + self.gap).max(1.0);
                let col = ((position.x() - origin.x()) / pitch).floor() as isize;
                let row = ((position.y() - origin.y()) / pitch).floor() as isize;
                if (0..17).contains(&col) && (0..23).contains(&row) {
                    let (x, y) = (col as usize, row as usize);
                    if !stepx_companion::device::is_backscreen_corner(x, y) {
                        ctx.dispatch_typed_action(cb(x, y));
                        return true;
                    }
                }
                false
            }
            _ => false,
        }
    }
    fn size(&self) -> Option<Vector2F> {
        self.size
    }
    fn origin(&self) -> Option<Point> {
        self.origin
    }
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::*;
    #[test]
    fn malformed_layout_rejects_invalid_bounds_without_hanging_hit_index() {
        let mut scene = facloud_ui::scene::Scene::new(1., Default::default());
        for i in 0..400 {
            scene.draw_rect_with_hit_recording(RectF::new(
                vec2f((i % 20) as f32, (i / 20) as f32),
                vec2f(1., 1.),
            ));
        }
        let started = std::time::Instant::now();
        let rejected = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            scene
                .draw_rect_with_hit_recording(RectF::new(vec2f(0., 0.), vec2f(f32::INFINITY, 23.)));
        }));
        assert!(rejected.is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }
}
