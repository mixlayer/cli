use super::*;
use serde_json::json;

fn expected(case: usize) -> String {
    std::fs::read_to_string(format!(
        "{}/parity/fixtures/go-output/expected/{case}.out",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
    .strip_suffix("exit=0\n")
    .unwrap()
    .into()
}

#[test]
fn recorded_model_output_cases_match_byte_for_byte() {
    let list = r#"{"object": "list", "data": [{"id": "fake/model", "object": "model"}]}"#;
    let item = r#"{"id": "fake/model", "object": "model"}"#;
    let flags = [
        Globals::default(),
        Globals {
            json: true,
            ..Default::default()
        },
        Globals {
            csv: true,
            ..Default::default()
        },
        Globals {
            plain: true,
            ..Default::default()
        },
        Globals {
            quiet: true,
            ..Default::default()
        },
        Globals {
            select: Some("id".into()),
            ..Default::default()
        },
        Globals {
            compact: true,
            ..Default::default()
        },
        Globals::default(),
        Globals {
            csv: true,
            ..Default::default()
        },
        Globals {
            quiet: true,
            ..Default::default()
        },
    ];
    for (index, g) in flags.iter().enumerate() {
        let result = format_endpoint(if index < 7 { list } else { item }, g, false, 0, false).unwrap();
        assert_eq!(result.stdout, expected(index + 1), "case {}", index + 1);
        assert!(result.stderr.is_empty());
    }
}

#[test]
fn recorded_explicit_tables_keep_headers_and_padding() {
    assert_eq!(
        table(&[
            vec!["MODEL".into(), "STATUS".into(), "OUTPUT USD/1M".into()],
            vec!["fake/model".into(), "".into(), "unknown".into()],
        ]),
        expected(15)
    );
    let profiles = table(&[
        vec!["NAME".into(), "FIELDS".into(), "DESCRIPTION".into()],
        vec!["p".into(), "1".into(), "d".into()],
    ]);
    for case in 20..=22 {
        assert_eq!(profiles, expected(case));
    }
}

#[test]
fn valid_chat_output_and_explicit_flag_precedence() {
    let raw = r#"{"id":"x","model":"fake/model","choices":[{"message":{"content":"[]"}}],"usage":{"prompt_tokens":1}}"#;
    let value: Value = serde_json::from_str(raw).unwrap();
    let g = Globals::default();
    let piped = format_endpoint(raw, &g, false, 1, false).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&piped.stdout).unwrap(),
        json!({"meta":{"source":"live"},"results":value})
    );
    let human = format_endpoint(raw, &g, true, 1, false).unwrap();
    assert_eq!(human.stderr, "1 results (live)\n");
    for terminal in [false, true] {
        assert_eq!(
            format_endpoint(
                raw,
                &Globals {
                    quiet: true,
                    ..g.clone()
                },
                terminal,
                1,
                false
            )
            .unwrap(),
            Rendered::default()
        );
        assert_eq!(
            format_endpoint(raw, &Globals { csv: true, ..g.clone() }, terminal, 1, false)
                .unwrap()
                .stdout,
            format!("{raw}\n")
        );
        let mixed = Globals {
            json: true,
            quiet: true,
            csv: true,
            ..g.clone()
        };
        assert_eq!(format_endpoint(raw, &mixed, terminal, 1, false).unwrap(), piped);
    }
}

#[test]
fn terminal_tables_provenance_and_projection_gates() {
    let raw = r#"[{"id":"one","status":"active","description":"long"}]"#;
    let g = Globals::default();
    let human = format_endpoint(raw, &g, true, 0, false).unwrap();
    assert_eq!(human.stdout, "ID   STATUS  DESCRIPTION\none  active  long\n");
    assert_eq!(human.stderr, "1 results (live)\n");
    for flags in [
        Globals {
            plain: true,
            ..g.clone()
        },
        Globals {
            compact: true,
            ..g.clone()
        },
        Globals {
            select: Some("id".into()),
            ..g.clone()
        },
        Globals { csv: true, ..g.clone() },
    ] {
        assert!(format_endpoint(raw, &flags, true, 0, false).unwrap().stderr.is_empty());
    }
    let g = Globals {
        select: Some("description".into()),
        compact: true,
        ..g
    };
    let selected = format_endpoint(raw, &g, false, 0, false).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&selected.stdout).unwrap()["results"],
        json!([{"description":"long"}])
    );
    let selected = format_endpoint(
        r#"{"data":[{"id":"one","z":2}]}"#,
        &Globals {
            select: Some("data.id".into()),
            ..Default::default()
        },
        false,
        0,
        false,
    )
    .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&selected.stdout).unwrap()["results"],
        json!([{"id":"one"}])
    );
}

#[test]
fn csv_array_escaping_nulls_and_go_nested_values() {
    let value = json!([
        {"id":"a,\"b\"\nnext","empty":null,"price":0.0,"nested":{"z":2,"a":1}},
        {"id":"second","price":1.5,"nested":["a","b"]}
    ]);
    let output = format_endpoint(
        &value.to_string(),
        &Globals {
            csv: true,
            ..Default::default()
        },
        false,
        0,
        false,
    )
    .unwrap();
    assert_eq!(
        output.stdout,
        "empty,id,nested,price\n,\"a,\"\"b\"\"\nnext\",map[a:1 z:2],0\n,second,[a b],1.5\n"
    );
}

#[test]
fn go_json_escaping_and_empty_select_preserve_output_contract() {
    let raw = r#"{"id": "<a&b>"}"#;
    let result = format_endpoint(raw, &Globals::default(), false, 0, false).unwrap();
    assert!(result.stdout.contains(r#""id": "\u003ca\u0026b\u003e""#));
    let g = Globals {
        csv: true,
        select: Some(" , ".into()),
        compact: true,
        ..Default::default()
    };
    assert_eq!(
        format_endpoint(raw, &g, false, 0, false).unwrap().stdout,
        format!("{raw}\n")
    );
    assert_eq!(
        filtered(
            Value::Null,
            &Globals {
                compact: true,
                ..Default::default()
            }
        ),
        json!([])
    );
}
