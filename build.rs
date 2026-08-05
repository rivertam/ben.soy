fn main() {
    // Watch the whole styles/ dir: input.css @imports the per-section files.
    println!("cargo:rerun-if-changed=styles");
    // Tailwind scans src/**/*.rs for utility classes, so a class first used
    // in a .rs file must rerun this script or it silently never reaches the
    // generated stylesheet. The scan honors .gitignore, so watch that too.
    println!("cargo:rerun-if-changed=src");
    // The scan's cwd covers crates/ too (diary-core's shared views carry
    // classes), but without this line an edit there would never rerun the
    // scan and the class would silently miss the stylesheet.
    println!("cargo:rerun-if-changed=crates/diary-core/src");
    println!("cargo:rerun-if-changed=.gitignore");
    topcoat::tailwind::BuildConfig::new()
        .input("styles/input.css")
        .render()
        .unwrap();
}
