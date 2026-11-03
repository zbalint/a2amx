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

fn parse_complete_blocks(markdown: &str, label: &str) -> usize {
    let mut parsed_blocks = 0;
    for block in toml_blocks(markdown) {
        if block.lines().next() == Some("# fragment") {
            continue;
        }
        let result = if block.lines().next() == Some("# global team file") {
            a2amx::team::parse_with(&block, Some("demo"))
        } else {
            a2amx::team::parse(&block)
        };
        if let Err(error) = result {
            panic!("{label} TOML did not parse: {error}\n{block}");
        }
        parsed_blocks += 1;
    }
    parsed_blocks
}

#[test]
fn toml_examples_in_the_confinement_guide_parse() {
    let guide = include_str!("../docs/confinement.md");
    let parsed_guide_blocks = parse_complete_blocks(guide, "confinement guide");
    assert!(
        parsed_guide_blocks > 0,
        "the confinement guide must contain a complete TOML example"
    );
    assert!(
        guide.lines().any(|line| line == "# global team file"),
        "the confinement guide must contain a global team file marker"
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

#[test]
fn toml_examples_in_getting_started_parse() {
    let guide = include_str!("../docs/getting-started.md");
    let parsed_blocks = parse_complete_blocks(guide, "getting-started guide");
    assert!(
        parsed_blocks >= 2,
        "the getting-started guide must contain at least two complete TOML examples, found {parsed_blocks}"
    );
}

#[test]
fn grant_table_matches_the_code() {
    let guide = include_str!("../docs/confinement.md");
    let mut in_grants = false;
    let mut doc_rows = Vec::new();

    for line in guide.lines() {
        if line == "### Grants" {
            in_grants = true;
            continue;
        }
        if in_grants && line.starts_with("### ") {
            break;
        }
        if !in_grants || !line.starts_with("| `") {
            continue;
        }

        let cells: Vec<_> = line.split('|').map(str::trim).collect();
        assert_eq!(cells.len(), 5, "malformed grant row: {line}");
        doc_rows.push((
            cells[1].trim_matches('`').to_owned(),
            cells[2].to_owned(),
            cells[3].to_owned(),
        ));
    }

    assert!(in_grants, "missing ### Grants heading");
    let code_grants = a2amx::grant::all();
    let doc_names: Vec<_> = doc_rows.iter().map(|row| row.0.as_str()).collect();
    let code_names: Vec<_> = code_grants.iter().map(|grant| grant.name).collect();
    assert_eq!(
        doc_names, code_names,
        "grant names differ: docs={doc_names:?}, code={code_names:?}"
    );

    for ((doc_name, doc_exposes, doc_default), code_grant) in doc_rows.iter().zip(code_grants) {
        let doc_exposes = doc_exposes.replace('`', "");
        let code_exposes = code_grant.exposes.replace('`', "");
        assert_eq!(
            doc_exposes, code_exposes,
            "grant `{doc_name}` exposes differ: docs={doc_exposes:?}, code={code_exposes:?}"
        );
        assert_eq!(
            doc_default.starts_with("Yes"),
            code_grant.team_default_ok,
            "grant `{doc_name}` default differs: docs={doc_default:?}, code team_default_ok={}",
            code_grant.team_default_ok
        );
        assert_eq!(
            doc_default.starts_with("No"),
            !code_grant.team_default_ok,
            "grant `{doc_name}` default must start with No when code team_default_ok is false"
        );
    }
}
