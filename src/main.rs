// Safety lints
#![deny(bare_trait_objects)]
#![deny(clippy::as_ptr_cast_mut)]
#![deny(clippy::cast_ptr_alignment)]
#![deny(clippy::large_stack_arrays)]
#![deny(clippy::ptr_as_ptr)]
#![deny(clippy::transmute_ptr_to_ptr)]
#![deny(clippy::unwrap_used)]
// Performance lints
#![warn(clippy::cloned_instead_of_copied)]
#![warn(clippy::inefficient_to_string)]
#![warn(clippy::invalid_upcast_comparisons)]
#![warn(clippy::iter_with_drain)]
#![warn(clippy::large_types_passed_by_value)]
#![warn(clippy::linkedlist)]
#![warn(clippy::mutex_integer)]
#![warn(clippy::naive_bytecount)]
#![warn(clippy::needless_bitwise_bool)]
#![warn(clippy::needless_collect)]
#![warn(clippy::needless_pass_by_value)]
#![warn(clippy::no_effect_underscore_binding)]
#![warn(clippy::or_fun_call)]
#![warn(clippy::stable_sort_primitive)]
#![warn(clippy::suboptimal_flops)]
#![warn(clippy::trivial_regex)]
#![warn(clippy::trivially_copy_pass_by_ref)]
#![warn(clippy::unnecessary_join)]
#![warn(clippy::unused_async)]
#![warn(clippy::zero_sized_map_values)]
// Correctness lints
#![deny(clippy::case_sensitive_file_extension_comparisons)]
#![deny(clippy::copy_iterator)]
#![deny(clippy::expl_impl_clone_on_copy)]
#![deny(clippy::float_cmp)]
#![warn(clippy::imprecise_flops)]
#![deny(clippy::manual_instant_elapsed)]
#![deny(clippy::match_same_arms)]
#![deny(clippy::mem_forget)]
#![warn(clippy::must_use_candidate)]
#![deny(clippy::path_buf_push_overwrite)]
#![deny(clippy::same_functions_in_if_condition)]
#![warn(clippy::suspicious_operation_groupings)]
#![deny(clippy::unchecked_duration_subtraction)]
#![deny(clippy::unicode_not_nfc)]
// Clarity/formatting lints
#![warn(clippy::borrow_as_ptr)]
#![warn(clippy::checked_conversions)]
#![warn(clippy::default_trait_access)]
#![warn(clippy::derive_partial_eq_without_eq)]
#![warn(clippy::explicit_deref_methods)]
#![warn(clippy::filter_map_next)]
#![warn(clippy::flat_map_option)]
#![warn(clippy::fn_params_excessive_bools)]
#![warn(clippy::from_iter_instead_of_collect)]
#![warn(clippy::if_not_else)]
#![warn(clippy::implicit_clone)]
#![warn(clippy::iter_not_returning_iterator)]
#![warn(clippy::iter_on_empty_collections)]
#![warn(clippy::macro_use_imports)]
#![warn(clippy::manual_clamp)]
#![warn(clippy::manual_let_else)]
#![warn(clippy::manual_ok_or)]
#![warn(clippy::manual_string_new)]
#![warn(clippy::map_flatten)]
#![warn(clippy::map_unwrap_or)]
#![warn(clippy::match_bool)]
#![warn(clippy::mut_mut)]
#![warn(clippy::needless_borrow)]
#![warn(clippy::needless_continue)]
#![warn(clippy::option_if_let_else)]
#![warn(clippy::range_minus_one)]
#![warn(clippy::range_plus_one)]
#![warn(clippy::redundant_else)]
#![warn(clippy::ref_binding_to_reference)]
#![warn(clippy::ref_option_ref)]
#![warn(clippy::semicolon_if_nothing_returned)]
#![warn(clippy::trait_duplication_in_bounds)]
#![warn(clippy::type_repetition_in_bounds)]
#![warn(clippy::unnested_or_patterns)]
#![warn(clippy::unused_peekable)]
#![warn(clippy::unused_rounding)]
#![warn(clippy::unused_self)]
#![warn(clippy::used_underscore_binding)]
#![warn(clippy::verbose_bit_mask)]
#![warn(clippy::verbose_file_reads)]
// Documentation lints
#![warn(clippy::doc_link_with_quotes)]
#![warn(clippy::doc_markdown)]
#![warn(clippy::missing_errors_doc)]
#![warn(clippy::missing_panics_doc)]

use std::{
    fs::File,
    io::{self, BufReader, Read, Write},
};

use anyhow::Result;
use av_scenechange::{detect_scene_changes, DetectionOptions};

use bpaf::Bpaf;

#[derive(Clone, Debug, Bpaf)]
#[bpaf(options)]
struct Args {
    /// Sets the input file to use
    pub input: String,

    /// Optional file to write results to
    #[bpaf(long, short)]
    pub output: Option<String>,

    /// Do not detect short scene flashes and exclude them as scene cuts
    pub no_flash_detection: bool,

    /// Sets a minimum interval between two consecutive scenecuts
    pub min_scenecut: Option<usize>,

    /// Sets a maximum interval between two consecutive scenecuts,
    /// after which a scenecut will be forced
    pub max_scenecut: Option<usize>,
}

fn main() -> Result<()> {
    let matches = args().run();
    let input = match matches.input.as_str() {
        "-" => Box::new(io::stdin()) as Box<dyn Read>,
        f => Box::new(File::open(f)?) as Box<dyn Read>,
    };
    let mut reader = BufReader::new(input);

    let opts = DetectionOptions {
        detect_flashes: !matches.no_flash_detection,
        min_scenecut_distance: matches.min_scenecut,
        max_scenecut_distance: matches.max_scenecut,
        ..DetectionOptions::default()
    };

    let mut dec = y4m::Decoder::new(&mut reader)?;
    let bit_depth = dec.get_bit_depth();
    let results = if bit_depth == 8 {
        detect_scene_changes::<_, u8>(&mut dec, opts, None, None)
    } else {
        detect_scene_changes::<_, u16>(&mut dec, opts, None, None)
    };
    println!("{}", serde_json::to_string(&results)?);

    if let Some(output_file) = matches.output {
        let mut file = File::create(output_file)?;

        let output = serde_json::to_string_pretty(&results)?;
        file.write_all(&output.into_bytes())?;
    }

    Ok(())
}
