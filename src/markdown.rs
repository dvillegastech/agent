/// Render markdown text for terminal display using termimad.
#[allow(dead_code)]
pub fn render(text: &str) -> String {
    // Use termimad to render markdown with terminal formatting
    let skin = termimad::MadSkin::default();
    let rendered = skin.text(text, None);
    // termimad::FmtText implements Display
    format!("{rendered}")
}
