//! Presentation-frame FPS, independent of fixed physics and HUD refresh cadence.
use bevy::prelude::*;

pub(crate) struct FpsOverlayPlugin;

#[derive(Component)]
struct FpsText;

#[derive(Default)]
struct FrameSample {
    frames: u32,
    seconds: f64,
}

impl Plugin for FpsOverlayPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn).add_systems(Update, update);
    }
}

fn spawn(mut commands: Commands) {
    // Use the existing gameplay camera; no independent overlay render loop.
    commands
        .spawn((
            Name::new("FPS overlay"),
            Node {
                position_type: PositionType::Absolute,
                top: px(12),
                right: px(12),
                padding: UiRect::axes(px(10), px(6)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
            GlobalZIndex(100),
        ))
        .with_children(|parent| {
            parent.spawn((
                FpsText,
                Text::new("FPS: --"),
                TextFont {
                    font_size: 20.0,
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
        });
}

fn update(
    time: Res<Time<Real>>,
    mut sample: Local<FrameSample>,
    mut labels: Query<&mut Text, With<FpsText>>,
) {
    let seconds = time.delta_secs_f64();
    if seconds <= 0.0 {
        return;
    }
    // One sample for EVERY presentation frame, including long/stalled frames.
    // Real time is unaffected by slow motion or the fixed simulation timestep.
    sample.frames += 1;
    sample.seconds += seconds;
    if sample.seconds >= 0.5 {
        let fps = f64::from(sample.frames) / sample.seconds;
        for mut text in &mut labels {
            text.0 = format!("FPS: {fps:.0}");
        }
        *sample = FrameSample::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn counter_tracks_render_updates_with_30_hz_physics_and_paused_game_time() {
        for fps in [30, 60, 120] {
            let mut app = App::new();
            app.insert_resource(Time::<Real>::default())
                .insert_resource(Time::<Fixed>::from_hz(30.0))
                .insert_resource(Time::<Virtual>::default())
                .add_systems(Update, update);
            app.world_mut().resource_mut::<Time<Virtual>>().pause();
            let label = app.world_mut().spawn((FpsText, Text::new("FPS: --"))).id();
            for _ in 0..fps * 2 {
                app.world_mut()
                    .resource_mut::<Time<Real>>()
                    .advance_by(Duration::from_secs_f64(1.0 / f64::from(fps)));
                app.update();
            }
            assert_eq!(app.world().get::<Text>(label).unwrap().0, format!("FPS: {fps}"));
        }
    }
}
