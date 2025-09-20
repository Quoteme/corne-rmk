use rmk::action::{EncoderAction, KeyAction};
use rmk::{k, lt, shifted};

// const ENTER_SHIFT: KeyAction = rmk::action::KeyAction::TapHold(
//     rmk::action::Action::Key(rmk::keycode::KeyCode::Enter),
//     rmk::action::Action::Key(rmk::keycode::KeyCode::LShift),
// );

const ENTER_SHIFT: KeyAction = k!(Enter);

const TRAN: KeyAction = rmk::action::KeyAction::Transparent;

const DOLLAR: KeyAction = shifted!(Kc4);

const PERCENTAGE: KeyAction = shifted!(Kc5);

const OPEN_BRACE: KeyAction = shifted!(Kc9);
const CLOSE_BRACE: KeyAction = shifted!(Kc0);
const OPEN_CURLY_BRACE: KeyAction = shifted!(LeftBracket);
const CLOSE_CURLY_BRACE: KeyAction = shifted!(RightBracket);
const UNDERLINE: KeyAction = shifted!(Minus);
const PLUS: KeyAction = shifted!(Equal);
const CARET: KeyAction = shifted!(Kc6);
const AMPERSAND: KeyAction = shifted!(Kc7);
const ASTERISK: KeyAction = shifted!(Kc8);
const TILDE: KeyAction = shifted!(Grave);
const HASHTAG: KeyAction = shifted!(Kc3);
const PIPE: KeyAction = shifted!(Backslash);

pub(crate) const COL: usize = 12;
pub(crate) const COL_OFFSET: usize = 6;
pub(crate) const ROW: usize = 4;
pub(crate) const NUM_LAYER: usize = 3;
pub(crate) const NUM_ENCODER: usize = 0;

#[rustfmt::skip]
pub const fn get_default_keymap() -> [[[KeyAction; COL]; ROW]; NUM_LAYER] {
    [
        [
            [k!(Tab),    k!(Q),  k!(W),  k!(E),    k!(R),        k!(T),     k!(Y),       k!(U),            k!(I),     k!(O),   k!(P),         k!(Backspace)],
            [k!(Escape), k!(A),  k!(S),  k!(D),    k!(F),        k!(G),     k!(H),       k!(J),            k!(K),     k!(L),   k!(Semicolon), k!(Quote) ],
            [k!(LShift), k!(Z),  k!(X),  k!(C),    k!(V),        k!(B),     k!(N),       k!(M),            k!(Comma), k!(Dot), k!(Slash),     k!(LAlt)  ],
            [k!(No),     k!(No), k!(No), k!(LGui), lt!(1,Slash), k!(Space), ENTER_SHIFT, lt!(2,Backspace), k!(LCtrl), k!(No),  k!(No),        k!(No)    ]
        ],
        [
            [k!(Tab),            k!(Kc1),     k!(Kc2),          k!(Kc3),           k!(Kc4),          k!(Kc5),         k!(Kc6),  k!(Kc7),          k!(Kc8),   k!(Kc9),   k!(Kc0), k!(Backspace)],
            [k!(BrightnessUp),   OPEN_BRACE,  k!(No),           DOLLAR,            k!(Backslash),    PERCENTAGE,      k!(Left), k!(Down),         k!(Up),    k!(Right), k!(No),  k!(No)],
            [k!(BrightnessDown), CLOSE_BRACE, OPEN_CURLY_BRACE, CLOSE_CURLY_BRACE, k!(LeftBracket), k!(RightBracket), k!(No),   k!(No),           k!(No),    k!(No),    k!(No),  k!(No)],
            [k!(No),             k!(No),      k!(No),           k!(LGui),          TRAN,             k!(Space),       TRAN,     lt!(2,Backspace), k!(LCtrl), k!(No),    k!(No),  k!(No)]
        ],
        [
            [k!(No), k!(No), k!(No), k!(No),     k!(No),     k!(No),      CARET,       AMPERSAND, ASTERISK,  k!(No),  k!(No), k!(No)],
            [k!(No), k!(No), k!(No), k!(No),     OPEN_BRACE, CLOSE_BRACE, k!(Minus),   PLUS,      k!(Grave), PIPE,    k!(No), k!(No)],
            [k!(No), k!(No), k!(No), k!(No),     k!(Escape), k!(Tab),     UNDERLINE,   k!(Equal), TILDE,     HASHTAG, k!(No), k!(No)],
            [k!(No), k!(No), k!(No), k!(LShift), TRAN,       k!(Space),   ENTER_SHIFT, TRAN,      k!(LCtrl), k!(No),  k!(No), k!(No)]
        ]
    ]
}

pub const fn get_default_encoder_map() -> [[EncoderAction; NUM_ENCODER]; NUM_LAYER] {
    [[], [], []]
}
