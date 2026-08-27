use vendetta_model::MessageState;

pub fn render_state_indicator(state: MessageState) -> Option<&'static str> {
    match state {
        MessageState::Deleted => Some(
            r##"<div class="state-badge badge-deleted"><svg class="icon"><use href="#icon-trash"></use></svg> <span>Deleted message</span></div>"##,
        ),
        MessageState::Empty => Some(
            r##"<div class="state-badge badge-empty"><svg class="icon"><use href="#icon-info"></use></svg> <span>Empty placeholder</span></div>"##,
        ),
        MessageState::Inaccessible => Some(
            r##"<div class="state-badge badge-inaccessible"><svg class="icon"><use href="#icon-lock"></use></svg> <span>Restricted/Inaccessible message</span></div>"##,
        ),
        MessageState::Active | MessageState::Edited => None,
    }
}
