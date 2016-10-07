
extern crate ufloat8;

include!(concat!(env!("OUT_DIR"), "/messages.rs"));

#[test]
fn test_color_conversion() {
    let color_raw = 0x4148ffff;
    let color = Color::from_raw(color_raw);
    if color.raw() != color_raw {
        panic!("color {:08x} turns into {:08x} after conversion to {:?}", color_raw, color.raw(), color);
    }
}
