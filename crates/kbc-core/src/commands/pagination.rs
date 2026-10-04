use crate::messages::{Messages, message};

pub const PAGE_SIZE: usize = 10;

pub enum Input {
    Select(usize),
    Move(Option<usize>),
    Finish,
    Other,
}

// 入力の仕様は文面キーから独立させ、検索とOC選択で同じ判定を使う。
pub fn parse(input: &str, page: usize) -> Input {
    let input = input.trim();
    let digits = |value: &str| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit());
    match input {
        "終了" | "取消" | "cancel" => Input::Finish,
        "次" => Input::Move(page.checked_add(1)),
        "前" => Input::Move(page.checked_sub(1)),
        _ if input.strip_suffix('p').is_some_and(digits) => Input::Move(
            input[..input.len() - 1]
                .parse::<usize>()
                .ok()
                .and_then(|page| page.checked_sub(1)),
        ),
        _ if digits(input) => Input::Select(
            input
                .parse::<usize>()
                .ok()
                .filter(|number| input == number.to_string())
                .unwrap_or(usize::MAX),
        ),
        _ => Input::Other,
    }
}

pub fn navigation(messages: &Messages, page: usize, pages: usize) -> String {
    let mut moves = Vec::new();
    if page + 1 < pages {
        moves.push(message!(messages, "navigation.next"));
    }
    if page > 0 {
        moves.push(message!(messages, "navigation.previous"));
    }
    message!(messages, "navigation.jump", moves = moves.join("　"))
}
