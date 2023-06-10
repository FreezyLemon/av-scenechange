// Copyright (c) 2018-2021, The rav1e contributors. All rights reserved
//
// This source code is subject to the terms of the BSD 2 Clause License and
// the Alliance for Open Media Patent License 1.0. If the BSD 2 Clause License
// was not distributed with this source code in the LICENSE file, you can
// obtain it at www.aomedia.org/license/software. If the Alliance for Open
// Media Patent License 1.0 was not distributed with this source code in the
// PATENTS file, you can obtain it at www.aomedia.org/license/patent.

mod fast;

use std::{sync::Arc, u64};

// use crate::api::{EncoderConfig};
// use crate::cpu_features::CpuFeatureLevel;
// use crate::encoder::Sequence;
// use crate::frame::*;
// use crate::me::RefMEStats;
// use crate::util::Pixel;
use debug_unreachable::debug_unreachable;
use v_frame::{frame::Frame, pixel::Pixel, plane::Plane};

use self::fast::{detect_scale_factor, FAST_THRESHOLD};
use crate::cpu_features::CpuFeatureLevel;

/// Experiments have determined this to be an optimal threshold
const IMP_BLOCK_DIFF_THRESHOLD: f64 = 7.0;

/// Fast integer division where divisor is a nonzero power of 2
#[inline(always)]
pub(crate) unsafe fn fast_idiv(n: usize, d: usize) -> usize {
    debug_assert!(d.is_power_of_two());

    // Remove branch on bsf instruction on x86 (which is used when compiling without
    // tzcnt enabled)
    if d == 0 {
        debug_unreachable!();
    }

    n >> d.trailing_zeros()
}

struct ScaleFunction<T: Pixel> {
    downscale_in_place: fn(/* &self: */ &Plane<T>, /* in_plane: */ &mut Plane<T>),
    downscale: fn(/* &self: */ &Plane<T>) -> Plane<T>,
    factor: usize,
}

impl<T: Pixel> ScaleFunction<T> {
    fn from_scale<const SCALE: usize>() -> Self {
        Self {
            downscale: Plane::downscale::<SCALE>,
            downscale_in_place: Plane::downscale_in_place::<SCALE>,
            factor: SCALE,
        }
    }
}
/// Runs keyframe detection on frames from the lookahead queue.
pub struct SceneChangeDetector<T: Pixel> {
    /// Minimum average difference between YUV deltas that will trigger a scene
    /// change.
    threshold: f64,
    /// Downscaling function for fast scene detection
    scale_func: Option<ScaleFunction<T>>,
    /// Frame buffer for scaled frames
    downscaled_frame_buffer: Option<(
        [Plane<T>; 2],
        // `true` if the data is valid and initialized, or `false`
        // if it should be assumed that the data is uninitialized.
        bool,
    )>,
    /// Frame buffer for holding references to source frames.
    ///
    /// Useful for not copying data into the downscaled frame buffer
    /// when using a downscale factor of 1.
    frame_ref_buffer: Option<[Arc<Frame<T>>; 2]>,
    /// Deque offset for current
    lookahead_offset: usize,
    /// Start deque offset based on lookahead
    deque_offset: usize,
    /// Scenechange results for adaptive threshold
    score_deque: Vec<ScenecutResult>,
    /// Number of pixels in scaled frame for fast mode
    pixels: usize,
    /// The bit depth of the video.
    bit_depth: usize,
    /// The CPU feature level to be used.
    cpu_feature_level: CpuFeatureLevel,

    min_kf_interval: u64,
    max_kf_interval: u64,
}

impl<T: Pixel> SceneChangeDetector<T> {
    pub fn new(
        bit_depth: usize,
        cpu_feature_level: CpuFeatureLevel,
        lookahead_distance: usize,
        max_frame_width: u32,
        max_frame_height: u32,
        min_kf_interval: u64,
        max_kf_interval: u64,
    ) -> Self {
        // Downscaling function for fast scene detection
        let scale_func = detect_scale_factor(max_frame_width, max_frame_height);

        // Set lookahead offset to 5 if normal lookahead available
        let lookahead_offset = if lookahead_distance >= 5 { 5 } else { 0 };
        let deque_offset = lookahead_offset;

        let score_deque = Vec::with_capacity(5 + lookahead_distance);

        // Downscaling factor for fast scenedetect (is currently always a power of 2)
        let factor = scale_func.as_ref().map_or(1, |x| x.factor);

        // SAFETY: factor should always be a power of 2 and not 0 because of
        // the output of detect_scale_factor.
        let pixels = unsafe {
            fast_idiv(max_frame_height as usize, factor)
                * fast_idiv(max_frame_width as usize, factor)
        };

        let threshold = FAST_THRESHOLD * (bit_depth as f64) / 8.0;

        Self {
            threshold,
            scale_func,
            downscaled_frame_buffer: None,
            frame_ref_buffer: None,
            lookahead_offset,
            deque_offset,
            score_deque,
            pixels,
            bit_depth,
            cpu_feature_level,
            min_kf_interval,
            max_kf_interval,
        }
    }

    /// Runs keyframe detection on the next frame in the lookahead queue.
    ///
    /// This function requires that a subset of input frames
    /// is passed to it in order, and that `keyframes` is only
    /// updated from this method. `input_frameno` should correspond
    /// to the second frame in `frame_set`.
    ///
    /// This will gracefully handle the first frame in the video as well.
    pub fn analyze_next_frame(
        &mut self,
        frame_set: &[&Arc<Frame<T>>],
        input_frameno: u64,
        previous_keyframe: u64,
    ) -> bool {
        // Use score deque for adaptive threshold for scene cut
        // Declare score_deque offset based on lookahead  for scene change scores

        // Find the distance to the previous keyframe.
        let distance = input_frameno - previous_keyframe;

        if frame_set.len() <= self.lookahead_offset {
            // Don't insert keyframes in the last few frames of the video
            // This is basically a scene flash and a waste of bits
            return false;
        }

        // Initialization of score deque
        // based on frame set length
        if self.deque_offset > 0
            && frame_set.len() > self.deque_offset + 1
            && self.score_deque.is_empty()
        {
            self.initialize_score_deque(frame_set, self.deque_offset);
        } else if self.score_deque.is_empty() {
            self.initialize_score_deque(frame_set, frame_set.len() - 1);

            self.deque_offset = frame_set.len() - 2;
        }
        // Running single frame comparison and adding it to deque
        // Decrease deque offset if there is no new frames
        if frame_set.len() > self.deque_offset + 1 {
            self.run_comparison(
                frame_set[self.deque_offset].clone(),
                frame_set[self.deque_offset + 1].clone(),
            );
        } else {
            self.deque_offset -= 1;
        }

        // Adaptive scenecut check
        let scenecut = self.adaptive_scenecut();
        let scenecut = self.handle_min_max_intervals(distance).unwrap_or(scenecut);
        #[cfg(feature = "devel")]
        debug!(
            "[SC-Detect] Frame {}: Raw={:5.1}  ImpBl={:5.1}  Bwd={:5.1}  Fwd={:5.1}  Th={:.1}  {}",
            input_frameno,
            score.inter_cost,
            score.imp_block_cost,
            score.backward_adjusted_cost,
            score.forward_adjusted_cost,
            score.threshold,
            if scenecut { "Scenecut" } else { "No cut" }
        );

        // Keep score deque of 5 backward frames
        // and forward frames of length of lookahead offset
        if self.score_deque.len() > 5 + self.lookahead_offset {
            self.score_deque.pop();
        }

        scenecut
    }

    fn handle_min_max_intervals(&mut self, distance: u64) -> Option<bool> {
        // Handle minimum and maximum keyframe intervals.
        if distance < self.min_kf_interval {
            return Some(false);
        }
        if distance >= self.max_kf_interval {
            return Some(true);
        }
        None
    }

    // Initially fill score deque with frame scores
    fn initialize_score_deque(&mut self, frame_set: &[&Arc<Frame<T>>], init_len: usize) {
        for x in 0..init_len {
            self.run_comparison(frame_set[x].clone(), frame_set[x + 1].clone());
        }
    }

    /// Runs scene change comparison beetween 2 given frames
    /// Insert result to start of score deque
    fn run_comparison(&mut self, frame1: Arc<Frame<T>>, frame2: Arc<Frame<T>>) {
        let result = self.fast_scenecut(frame1, frame2);
        self.score_deque.insert(0, result);
    }

    /// Compares current scene score to adapted threshold based on previous
    /// scores Value of current frame is offset by lookahead, if lookahead
    /// >=5 Returns true if current scene score is higher than adapted
    /// threshold
    fn adaptive_scenecut(&mut self) -> bool {
        let score = self.score_deque[self.deque_offset];

        // We use the importance block algorithm's cost metrics as a secondary algorithm
        // because, although it struggles in certain scenarios such as
        // finding the end of a pan, it is very good at detecting hard scenecuts
        // or detecting if a pan exists.
        // Because of this, we only consider a frame for a scenechange if
        // the importance block algorithm is over the threshold either on this frame
        // (hard scenecut) or within the past few frames (pan). This helps
        // filter out a few false positives produced by the cost-based
        // algorithm.
        let imp_block_threshold = IMP_BLOCK_DIFF_THRESHOLD * (self.bit_depth as f64) / 8.0;
        if !&self.score_deque[self.deque_offset..]
            .iter()
            .any(|result| result.imp_block_cost >= imp_block_threshold)
        {
            return false;
        }

        let cost = score.forward_adjusted_cost;
        if cost >= score.threshold {
            let back_deque = &self.score_deque[self.deque_offset + 1..];
            let forward_deque = &self.score_deque[..self.deque_offset];
            let back_over_tr_count = back_deque
                .iter()
                .filter(|result| result.backward_adjusted_cost >= result.threshold)
                .count();
            let forward_over_tr_count = forward_deque
                .iter()
                .filter(|result| result.forward_adjusted_cost >= result.threshold)
                .count();

            // Check for scenecut after the flashes
            // No frames over threshold forward
            // and some frames over threshold backward
            // Fast scenecut is more sensitive to false flash detection,
            // so we want more "evidence" of there being a flash before creating a keyframe.
            let back_count_req = 2;
            if forward_over_tr_count == 0 && back_over_tr_count >= back_count_req {
                return true;
            }

            // Check for scenecut before flash
            // If distance longer than max flash length
            if back_over_tr_count == 0
                && forward_over_tr_count == 1
                && forward_deque[0].forward_adjusted_cost >= forward_deque[0].threshold
            {
                return true;
            }

            if back_over_tr_count != 0 || forward_over_tr_count != 0 {
                return false;
            }
        }

        cost >= score.threshold
    }
}

#[derive(Debug, Clone, Copy)]
struct ScenecutResult {
    imp_block_cost: f64,
    backward_adjusted_cost: f64,
    forward_adjusted_cost: f64,
    threshold: f64,
}
