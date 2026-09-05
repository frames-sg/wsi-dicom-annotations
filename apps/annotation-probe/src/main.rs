#![forbid(unsafe_code)]

use peak_alloc::PeakAlloc;

mod command;
mod conversion_report;
mod convert_geojson;
mod convert_raster;
mod legacy;
mod producer;
mod publication;

#[global_allocator]
pub(crate) static PEAK_ALLOC: PeakAlloc = PeakAlloc;

fn main() {
    let exit_code = command::execute(
        std::env::args_os().skip(1),
        std::io::stdout().lock(),
        std::io::stderr().lock(),
    );
    if exit_code != 0 {
        std::process::exit(exit_code);
    }
}
