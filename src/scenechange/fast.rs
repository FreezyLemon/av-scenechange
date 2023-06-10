use std::{cmp, sync::Arc};

use debug_unreachable::debug_unreachable;
use v_frame::{frame::Frame, pixel::Pixel, plane::Plane};

use super::{ScaleFunction, SceneChangeDetector, ScenecutResult};
use crate::sad_plane;

/// Experiments have determined this to be an optimal threshold
pub(super) const FAST_THRESHOLD: f64 = 18.0;

impl<T: Pixel> SceneChangeDetector<T> {
    /// The fast algorithm detects fast cuts using a raw difference
    /// in pixel values between the scaled frames.
    pub(super) fn fast_scenecut(
        &mut self,
        frame1: Arc<Frame<T>>,
        frame2: Arc<Frame<T>>,
    ) -> ScenecutResult {
        if let Some(scale_func) = &self.scale_func {
            // downscale both frames for faster comparison
            if let Some((frame_buffer, is_initialized)) = &mut self.downscaled_frame_buffer {
                let frame_buffer = &mut *frame_buffer;
                if *is_initialized {
                    frame_buffer.swap(0, 1);
                    (scale_func.downscale_in_place)(&frame2.planes[0], &mut frame_buffer[1]);
                } else {
                    // both frames are in an irrelevant and invalid state, so we have to
                    // reinitialize them, but we can reuse their allocations
                    (scale_func.downscale_in_place)(&frame1.planes[0], &mut frame_buffer[0]);
                    (scale_func.downscale_in_place)(&frame2.planes[0], &mut frame_buffer[1]);
                    *is_initialized = true;
                }
            } else {
                self.downscaled_frame_buffer = Some((
                    [
                        (scale_func.downscale)(&frame1.planes[0]),
                        (scale_func.downscale)(&frame2.planes[0]),
                    ],
                    true, // the frame buffer is initialized and in a valid state
                ));
            }

            if let Some((frame_buffer, _)) = &self.downscaled_frame_buffer {
                let &[first, second] = &frame_buffer;
                let delta = self.delta_in_planes(first, second);

                ScenecutResult {
                    threshold: self.threshold,
                    imp_block_cost: delta,
                    forward_adjusted_cost: delta,
                    backward_adjusted_cost: delta,
                }
            } else {
                // SAFETY: `downscaled_frame_buffer` is always initialized to `Some(..)` with a
                // valid state before this if/else block is reached.
                unsafe { debug_unreachable!() }
            }
        } else {
            if let Some(frame_buffer) = &mut self.frame_ref_buffer {
                frame_buffer.swap(0, 1);
                frame_buffer[1] = frame2;
            } else {
                self.frame_ref_buffer = Some([frame1, frame2]);
            }

            if let Some(frame_buffer) = &self.frame_ref_buffer {
                let delta =
                    self.delta_in_planes(&frame_buffer[0].planes[0], &frame_buffer[1].planes[0]);

                ScenecutResult {
                    threshold: self.threshold,
                    imp_block_cost: delta,
                    backward_adjusted_cost: delta,
                    forward_adjusted_cost: delta,
                }
            } else {
                // SAFETY: `frame_ref_buffer` is always initialized to `Some(..)` at the start
                // of this code block if it was `None`.
                unsafe { debug_unreachable!() }
            }
        }
    }

    /// Calculates the average sum of absolute difference (SAD) per pixel
    /// between 2 planes
    fn delta_in_planes(&self, plane1: &Plane<T>, plane2: &Plane<T>) -> f64 {
        let delta = sad_plane::sad_plane(plane1, plane2, self.cpu_feature_level);

        delta as f64 / self.pixels as f64
    }
}

/// Scaling factor for frame in scene detection
pub(super) fn detect_scale_factor<T: Pixel>(
    max_width: u32,
    max_height: u32,
) -> Option<ScaleFunction<T>> {
    let small_edge = cmp::min(max_height, max_width);
    match small_edge {
        0..=240 => None,
        241..=480 => Some(ScaleFunction::from_scale::<2>()),
        481..=720 => Some(ScaleFunction::from_scale::<4>()),
        721..=1080 => Some(ScaleFunction::from_scale::<8>()),
        1081..=1600 => Some(ScaleFunction::from_scale::<16>()),
        1601..=u32::MAX => Some(ScaleFunction::from_scale::<32>()),
    }
}
