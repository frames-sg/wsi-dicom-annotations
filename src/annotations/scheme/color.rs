use crate::{Error, Result};

pub(super) fn parse_rgb(value: &str) -> Result<[u8; 3]> {
    if value.len() != 7 || !value.starts_with('#') {
        return Err(Error::InvalidInput(
            "annotation class display_color must use #RRGGBB".into(),
        ));
    }
    let parse = |range: std::ops::Range<usize>| {
        u8::from_str_radix(&value[range], 16).map_err(|_| {
            Error::InvalidInput("annotation class display_color must use #RRGGBB".into())
        })
    };
    Ok([parse(1..3)?, parse(3..5)?, parse(5..7)?])
}

#[must_use]
pub fn srgb_to_dicom_cielab(rgb: [u8; 3]) -> [u16; 3] {
    let linear = rgb.map(|value| srgb_channel_to_linear(f64::from(value) / 255.0));
    let xyz_d65 = [
        0.412_456_4 * linear[0] + 0.357_576_1 * linear[1] + 0.180_437_5 * linear[2],
        0.212_672_9 * linear[0] + 0.715_152_2 * linear[1] + 0.072_175 * linear[2],
        0.019_333_9 * linear[0] + 0.119_192 * linear[1] + 0.950_304_1 * linear[2],
    ];
    let xyz = [
        1.047_929_8 * xyz_d65[0] + 0.022_946_8 * xyz_d65[1] - 0.050_192_2 * xyz_d65[2],
        0.029_627_8 * xyz_d65[0] + 0.990_434_5 * xyz_d65[1] - 0.017_073_8 * xyz_d65[2],
        -0.009_243 * xyz_d65[0] + 0.015_055_2 * xyz_d65[1] + 0.751_874_3 * xyz_d65[2],
    ];
    let f = [
        lab_f(xyz[0] / 0.964_2),
        lab_f(xyz[1]),
        lab_f(xyz[2] / 0.824_9),
    ];
    let l = 116.0 * f[1] - 16.0;
    let a = 500.0 * (f[0] - f[1]);
    let b = 200.0 * (f[1] - f[2]);
    [
        encode_lab_component(l, 100.0, 0.0),
        encode_lab_component(a, 255.0, 128.0),
        encode_lab_component(b, 255.0, 128.0),
    ]
}

#[must_use]
pub fn dicom_cielab_to_srgb(lab: [u16; 3]) -> [u8; 3] {
    let l = f64::from(lab[0]) * 100.0 / 65_535.0;
    let a = f64::from(lab[1]) * 255.0 / 65_535.0 - 128.0;
    let b = f64::from(lab[2]) * 255.0 / 65_535.0 - 128.0;
    let fy = (l + 16.0) / 116.0;
    let fx = fy + a / 500.0;
    let fz = fy - b / 200.0;
    let xyz_d50 = [
        0.964_2 * lab_f_inverse(fx),
        lab_f_inverse(fy),
        0.824_9 * lab_f_inverse(fz),
    ];
    let xyz = [
        0.955_473_4 * xyz_d50[0] - 0.023_098_5 * xyz_d50[1] + 0.063_259_3 * xyz_d50[2],
        -0.028_369_7 * xyz_d50[0] + 1.009_995_5 * xyz_d50[1] + 0.021_041_4 * xyz_d50[2],
        0.012_314 * xyz_d50[0] - 0.020_507_7 * xyz_d50[1] + 1.330_365_9 * xyz_d50[2],
    ];
    let linear = [
        3.240_454_2 * xyz[0] - 1.537_138_5 * xyz[1] - 0.498_531_4 * xyz[2],
        -0.969_266 * xyz[0] + 1.876_010_8 * xyz[1] + 0.041_556 * xyz[2],
        0.055_643_4 * xyz[0] - 0.204_025_9 * xyz[1] + 1.057_225_2 * xyz[2],
    ];
    linear.map(|value| (linear_channel_to_srgb(value).clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn srgb_channel_to_linear(value: f64) -> f64 {
    if value <= 0.040_45 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_channel_to_srgb(value: f64) -> f64 {
    if value <= 0.003_130_8 {
        12.92 * value
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

fn lab_f(value: f64) -> f64 {
    const EPSILON: f64 = 216.0 / 24_389.0;
    const KAPPA: f64 = 24_389.0 / 27.0;
    if value > EPSILON {
        value.cbrt()
    } else {
        (KAPPA * value + 16.0) / 116.0
    }
}

fn lab_f_inverse(value: f64) -> f64 {
    const EPSILON: f64 = 216.0 / 24_389.0;
    const KAPPA: f64 = 24_389.0 / 27.0;
    let cube = value * value * value;
    if cube > EPSILON {
        cube
    } else {
        (116.0 * value - 16.0) / KAPPA
    }
}

fn encode_lab_component(value: f64, range: f64, offset: f64) -> u16 {
    (((value + offset) / range).clamp(0.0, 1.0) * 65_535.0).round() as u16
}
