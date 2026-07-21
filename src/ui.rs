use console::Style;

pub fn error(err: &anyhow::Error) {
    let style = Style::new().for_stderr().red().bold();
    let dim = Style::new().for_stderr().dim();
    eprintln!("{} {}", style.apply_to("✗ error:"), err);
    for cause in err.chain().skip(1) {
        eprintln!("  {}", dim.apply_to(cause));
    }
}

pub fn step(message: &str) {
    let style = Style::new().for_stderr().cyan().bold();
    eprintln!("{} {message}", style.apply_to("→"));
}

pub fn warn(message: &str) {
    let style = Style::new().for_stderr().yellow().bold();
    eprintln!("{} {message}", style.apply_to("⚠"));
}

pub fn success(message: &str) {
    let style = Style::new().for_stderr().green().bold();
    eprintln!("{} {message}", style.apply_to("✓"));
}
