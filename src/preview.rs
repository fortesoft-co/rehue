//! The HTML preview (`preview.html`): a compiled-in, script-free page
//! that renders a palette as terminal / code / control mocks plus two
//! widget suites mirroring real theme wiring.  GTK view = the

//! adw-gtk3 + `@define-color` css that stylix ships (accent base0D,

//! success base0B, destructive base08, chrome base01); Qt view = stylix's `kvconfig.mustache` colour
//! mapping, exactly (window base01, button base02, highlight base0E
//! with text base00, link base0D, visited base0E).  Views toggle with
//! pure CSS (radio + sibling selectors); derived tints ride
//! `color-mix`; every byte is deterministic.

/// The template, next to this source file in `src/`.
const TEMPLATE: &str = include_str!("preview-template.html");

/// Escape the variable text (scheme names); the palette hexes are
/// already normalized.
fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Render the preview for 16 slot hexes in canonical base order.
/// Unfilled or leftover placeholders fail closed.
pub fn render(title: &str, hexes: &[String]) -> Result<String, String> {
    if hexes.len() != 16 {
        return Err(format!(
            "the preview renders 16 slots (canonical base order), got {}",
            hexes.len()
        ));
    }
    let mut html = TEMPLATE.to_string();
    html = html.replace("{{title}}", &escape_html(title));
    for (index, hex) in hexes.iter().enumerate() {
        html = html.replace(&format!("{{{{base{:02x}}}}}", index), hex);
    }
    if html.contains("{{") {
        return Err("preview template has unfilled placeholders".to_string());
    }
    Ok(html)
}
