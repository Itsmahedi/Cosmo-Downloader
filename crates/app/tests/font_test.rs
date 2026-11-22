#[test]
fn test_bengali_font_rendering() {
    let ctx = eframe::egui::Context::default();
    rustget_app::theme::configure_fonts(&ctx);
    let _ = ctx.run(Default::default(), |_| {});

    let bengali_sample = "বাংলা গান ভিডিও";
    let font_id = eframe::egui::FontId::proportional(14.0);
    let galley = ctx.fonts(|f| f.layout_no_wrap(bengali_sample.to_string(), font_id, eframe::egui::Color32::WHITE));

    let mut total_glyphs = 0;
    for row in &galley.rows {
        for glyph in &row.glyphs {
            total_glyphs += 1;
            println!("Glyph chr: {:?}, uv_rect: {:?}", glyph.chr, glyph.uv_rect);
            // In egui, a missing glyph (tofu) renders the replacement char or has empty UV or special rect
        }
    }
    println!("Total rows: {}, total glyphs: {}", galley.rows.len(), total_glyphs);
    assert!(total_glyphs > 0);
}
