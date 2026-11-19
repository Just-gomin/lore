//! 내장 스킬과 안내문에 나오는 명령과 입력 예시가 지금의 lore에서 그대로 통하는지 확인한다.
//!
//! 명령의 인자가 바뀌면 이 테스트가 실패해, 스킬도 같은 PR에서 고치게 한다.

use lore::cli::{self, Invocation};
use lore::install::{AGENTS_BODY, INIT_SKILLS, SKILLS};

/// 마크다운에서 `lang` 코드블록의 내용을 모은다.
fn code_blocks(text: &str, lang: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        match &mut current {
            None if line.trim() == format!("```{lang}") => current = Some(String::new()),
            Some(block) if line.trim() == "```" => {
                blocks.push(std::mem::take(block));
                current = None;
            }
            Some(block) => {
                block.push_str(line);
                block.push('\n');
            }
            None => {}
        }
    }
    blocks
}

/// 큰따옴표만 다루는 간단한 셸 낱말 나누기. `#` 뒤는 주석이고, `<파일 경로 | ID>` 같은 자리 표시는 한 낱말이다.
fn shell_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    let mut placeholder = false;
    let mut started = false;
    for ch in line.chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            '<' if !quoted && !started => {
                placeholder = true;
                word.push(ch);
                started = true;
            }
            '>' if placeholder => {
                placeholder = false;
                word.push(ch);
            }
            ' ' if placeholder => word.push(ch),
            '#' if !quoted && !started => break,
            ' ' if !quoted => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            _ => {
                word.push(ch);
                started = true;
            }
        }
    }
    if started {
        words.push(word);
    }
    words
}

fn sh_commands(text: &str) -> Vec<Vec<String>> {
    code_blocks(text, "sh")
        .iter()
        .flat_map(|block| block.lines().map(shell_words).collect::<Vec<_>>())
        .filter(|words| words.first().map(String::as_str) == Some("lore"))
        // 안내문의 `<파일 경로 | ID>` 같은 자리 표시는 실제 값으로 바꿔 본다.
        .map(|words| {
            words
                .into_iter()
                .skip(1)
                .map(|word| {
                    if word.starts_with('<') {
                        "x".to_string()
                    } else {
                        word
                    }
                })
                .collect()
        })
        .collect()
}

#[test]
fn 스킬과_안내문의_lore_명령은_구현된_명령으로_해석된다() {
    let mut texts: Vec<(&str, &str)> = SKILLS.iter().chain(&INIT_SKILLS).copied().collect();
    texts.push(("AGENTS.md 구간", AGENTS_BODY));
    let mut checked = 0;
    for (name, text) in texts {
        for args in sh_commands(text) {
            match cli::parse(&args) {
                Ok(Invocation::Run { .. }) => {}
                other => panic!("{name}: {args:?}를 해석하지 못했습니다: {other:?}"),
            }
            checked += 1;
        }
    }
    assert!(checked >= 8, "확인한 명령이 너무 적습니다: {checked}");
}

#[test]
fn 소스_노트_스킬의_입력_예시는_입력_검사를_통과한다() {
    let (_, text) = SKILLS
        .iter()
        .find(|(name, _)| *name == "lore-source")
        .unwrap();
    let examples = code_blocks(text, "json");
    assert_eq!(examples.len(), 1);
    lore::add_source::parse_input("lore-source", &examples[0]).unwrap();
    // 추출기 예시는 표준 입력으로 넘기는 JSON이다. 꼭 있어야 할 항목이 들었는지 본다.
    let extractor = code_blocks(text, "sh")
        .into_iter()
        .find(|block| block.contains("lore-extractor-"))
        .expect("추출기 예시가 있다");
    let json = extractor
        .split('\'')
        .nth(1)
        .expect("작은따옴표로 감싼 입력 JSON");
    let input: serde_json::Value = serde_json::from_str(json).unwrap();
    for key in ["file", "channel", "origin"] {
        assert!(input[key].is_string(), "{key}");
    }
}

#[test]
fn 설치하는_마크다운의_중첩_목록은_두_칸_들여쓰기다() {
    // 설치한 레포의 포매터(prettier 등)가 들여쓰기를 바꾸면 스킬 해시가 어긋나
    // 사람이 고친 스킬로 보이게 된다. prettier의 기본 형식인 두 칸으로 쓴다.
    let mut texts: Vec<(&str, &str)> = SKILLS.iter().chain(&INIT_SKILLS).copied().collect();
    texts.push(("AGENTS.md 구간", AGENTS_BODY));
    for (name, text) in texts {
        let mut in_code = false;
        for (number, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("```") {
                in_code = !in_code;
            }
            let indent = line.len() - line.trim_start().len();
            let is_item = line.trim_start().starts_with("- ");
            assert!(
                in_code || !is_item || indent % 2 == 0 && indent <= 2,
                "{name} {}번째 줄의 목록 들여쓰기가 두 칸이 아닙니다: {line:?}",
                number + 1
            );
        }
    }
}
