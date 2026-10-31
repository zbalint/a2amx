fn toml_blocks(markdown: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut lines = markdown.lines();
    while let Some(line) = lines.next() {
        if line != "```toml" {
            continue;
        }
        let mut block = String::new();
        let mut closed = false;
        for line in lines.by_ref() {
            if line == "```" {
                closed = true;
                break;
            }
            block.push_str(line);
            block.push('\n');
        }
        assert!(closed, "unterminated TOML fence");
        blocks.push(block);
    }
    blocks
}

#[test]
fn toml_examples_in_the_confinement_guide_parse() {
    let guide = include_str!("../docs/confinement.md");
    let mut parsed_guide_blocks = 0;
    for block in toml_blocks(guide) {
        if block.lines().next() == Some("# fragment") {
            continue;
        }
        if let Err(error) = a2amx::team::parse(&block) {
            panic!("confinement guide TOML did not parse: {error}\n{block}");
        }
        parsed_guide_blocks += 1;
    }
    assert!(
        parsed_guide_blocks > 0,
        "the confinement guide must contain a complete TOML example"
    );

    let example = include_str!("../a2amx.toml.example");
    let mut in_confinement_block = false;
    let mut block = Vec::new();
    let mut other_lines = Vec::new();
    for line in example.lines() {
        match line {
            "# --- confinement example ---" => {
                assert!(!in_confinement_block, "nested confinement example marker");
                in_confinement_block = true;
            }
            "# --- end confinement example ---" => {
                assert!(in_confinement_block, "end marker without start");
                in_confinement_block = false;
            }
            _ if in_confinement_block => block.push(line),
            _ => other_lines.push(line),
        }
    }
    assert!(!in_confinement_block, "unterminated confinement example");
    assert!(!block.is_empty(), "missing confinement example block");

    for line in block {
        let Some(uncommented) = line.strip_prefix("# ") else {
            panic!("confinement example line is not commented: {line}");
        };
        other_lines.push(uncommented);
    }
    let reconstructed = other_lines.join("\n");
    if let Err(error) = a2amx::team::parse(&reconstructed) {
        panic!("a2amx.toml.example confinement block did not parse: {error}");
    }
}
