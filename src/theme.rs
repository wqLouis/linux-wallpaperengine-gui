use iced::{
    Background, Border, Color, Shadow, Vector,
    widget::{button, container, scrollable, text_input},
};

// ── Color Palette ──────────────────────────────────────────────────────────
// Deep charcoal background with cool undertones

pub const BG_DEEP: Color = Color::from_rgb(0.055, 0.055, 0.078);   // #0e0e14
pub const BG_SURFACE: Color = Color::from_rgb(0.078, 0.078, 0.106); // #14141b
pub const BG_CARD: Color = Color::from_rgb(0.102, 0.102, 0.137);    // #1a1a23
pub const BG_ELEVATED: Color = Color::from_rgb(0.149, 0.149, 0.192); // #262631

pub const ACCENT: Color = Color::from_rgb(0.506, 0.227, 0.929);     // #813ae9
pub const ACCENT_HOVER: Color = Color::from_rgb(0.580, 0.318, 0.953); // #9451f3
pub const ACCENT_MUTED: Color = Color::from_rgb(0.325, 0.149, 0.600); // #532699

pub const SUCCESS: Color = Color::from_rgb(0.133, 0.773, 0.369);    // #22c55e
pub const ERROR: Color = Color::from_rgb(0.937, 0.267, 0.267);      // #ef4444
pub const WARNING: Color = Color::from_rgb(0.957, 0.643, 0.102);    // #f4a40f

pub const TEXT_PRIMARY: Color = Color::from_rgb(0.925, 0.933, 0.973); // #ecedf8
pub const TEXT_SECONDARY: Color = Color::from_rgb(0.631, 0.651, 0.714); // #a1a6b6
pub const TEXT_MUTED: Color = Color::from_rgb(0.396, 0.408, 0.463);  // #656876
pub const TEXT_ACCENT: Color = Color::from_rgb(0.725, 0.604, 0.984); // #b99afb

pub const BORDER_SUBTLE: Color = Color::from_rgb(0.165, 0.165, 0.208); // #2a2a35

// ── Border radii ───────────────────────────────────────────────────────────

#[allow(dead_code)]
pub const R_SM: f32 = 6.0;
pub const R_MD: f32 = 10.0;
pub const R_LG: f32 = 14.0;
pub const R_FULL: f32 = 999.0;

// ── Shadows ─────────────────────────────────────────────────────────────────

fn shadow_subtle() -> Shadow {
    Shadow {
        color: Color::from_rgba(0.0, 0.0, 0.0, 0.25),
        offset: Vector::new(0.0, 2.0),
        blur_radius: 8.0,
    }
}

fn shadow_card() -> Shadow {
    Shadow {
        color: Color::from_rgba(0.0, 0.0, 0.0, 0.35),
        offset: Vector::new(0.0, 4.0),
        blur_radius: 16.0,
    }
}

fn shadow_card_hover() -> Shadow {
    Shadow {
        color: Color::from_rgba(0.506, 0.227, 0.929, 0.20),
        offset: Vector::new(0.0, 8.0),
        blur_radius: 28.0,
    }
}

fn shadow_button() -> Shadow {
    Shadow {
        color: Color::from_rgba(0.506, 0.227, 0.929, 0.35),
        offset: Vector::new(0.0, 3.0),
        blur_radius: 10.0,
    }
}

fn shadow_none() -> Shadow {
    Shadow {
        color: Color::TRANSPARENT,
        offset: Vector::new(0.0, 0.0),
        blur_radius: 0.0,
    }
}

// ── Border helpers ──────────────────────────────────────────────────────────

fn border_subtle(radius: f32) -> Border {
    Border {
        color: BORDER_SUBTLE,
        width: 1.0,
        radius: radius.into(),
    }
}

fn border_glow(radius: f32) -> Border {
    Border {
        color: Color::from_rgba(0.506, 0.227, 0.929, 0.3),
        width: 1.5,
        radius: radius.into(),
    }
}

fn border_none() -> Border {
    Border {
        color: Color::TRANSPARENT,
        width: 0.0,
        radius: 0.0.into(),
    }
}

fn border_accent(radius: f32) -> Border {
    Border {
        color: Color::from_rgba(0.506, 0.227, 0.929, 0.2),
        width: 1.0,
        radius: radius.into(),
    }
}

// ── Custom Button Styles ────────────────────────────────────────────────────

pub fn btn_primary(_theme: &iced::Theme, status: button::Status) -> button::Style {
    let (bg, text_c, border, shadow, snap) = match status {
        button::Status::Active => (
            Some(Background::Color(ACCENT)),
            Color::WHITE,
            Border { color: Color::TRANSPARENT, width: 0.0, radius: R_MD.into() },
            shadow_button(),
            false,
        ),
        button::Status::Hovered => (
            Some(Background::Color(ACCENT_HOVER)),
            Color::WHITE,
            border_glow(R_MD),
            shadow_card_hover(),
            false,
        ),
        button::Status::Pressed => (
            Some(Background::Color(ACCENT_MUTED)),
            Color::WHITE,
            Border { color: ACCENT_MUTED, width: 2.0, radius: R_MD.into() },
            shadow_none(),
            false,
        ),
        button::Status::Disabled => (
            Some(Background::Color(BG_SURFACE)),
            TEXT_MUTED,
            border_subtle(R_MD),
            shadow_none(),
            false,
        ),
    };

    button::Style { background: bg, text_color: text_c, border, shadow, snap }
}

pub fn btn_secondary(_theme: &iced::Theme, status: button::Status) -> button::Style {
    let (bg, border, shadow) = match status {
        button::Status::Active => (
            Some(Background::Color(BG_CARD)),
            border_subtle(R_MD),
            shadow_subtle(),
        ),
        button::Status::Hovered => (
            Some(Background::Color(BG_ELEVATED)),
            border_glow(R_MD),
            shadow_card(),
        ),
        button::Status::Pressed => (
            Some(Background::Color(BG_DEEP)),
            border_subtle(R_MD),
            shadow_none(),
        ),
        button::Status::Disabled => (
            Some(Background::Color(BG_SURFACE)),
            border_subtle(R_MD),
            shadow_none(),
        ),
    };

    let text_c = if status == button::Status::Disabled { TEXT_MUTED } else { TEXT_PRIMARY };

    button::Style { background: bg, text_color: text_c, border, shadow, snap: false }
}

pub fn btn_nav(_theme: &iced::Theme, status: button::Status) -> button::Style {
    let (bg, text_c, border) = match status {
        button::Status::Active => (
            None,
            TEXT_SECONDARY,
            border_accent(R_FULL),
        ),
        button::Status::Hovered => (
            Some(Background::Color(Color::from_rgba(0.506, 0.227, 0.929, 0.15))),
            TEXT_ACCENT,
            border_accent(R_FULL),
        ),
        button::Status::Pressed => (
            Some(Background::Color(ACCENT_MUTED)),
            Color::WHITE,
            Border { color: Color::TRANSPARENT, width: 0.0, radius: R_FULL.into() },
        ),
        button::Status::Disabled => (
            None,
            TEXT_MUTED,
            Border { color: Color::TRANSPARENT, width: 0.0, radius: R_FULL.into() },
        ),
    };

    button::Style { background: bg, text_color: text_c, border, shadow: shadow_none(), snap: false }
}

/// Active (current) nav button style — always shows as filled accent
pub fn btn_nav_active(_theme: &iced::Theme, _status: button::Status) -> button::Style {
    button::Style {
        background: Some(Background::Color(ACCENT)),
        text_color: Color::WHITE,
        border: Border { color: Color::TRANSPARENT, width: 0.0, radius: R_FULL.into() },
        shadow: shadow_button(),
        snap: false,
    }
}

// ── Custom Container Styles ─────────────────────────────────────────────────

pub fn card_style(_theme: &iced::Theme) -> container::Style {
    container::Style {
        text_color: Some(TEXT_PRIMARY),
        background: Some(Background::Color(BG_CARD)),
        border: border_subtle(R_LG),
        shadow: shadow_card(),
        snap: false,
    }
}

pub fn surface_style(_theme: &iced::Theme) -> container::Style {
    container::Style {
        text_color: Some(TEXT_PRIMARY),
        background: Some(Background::Color(BG_SURFACE)),
        border: border_subtle(R_MD),
        shadow: shadow_subtle(),
        snap: false,
    }
}

pub fn top_bar_style(_theme: &iced::Theme) -> container::Style {
    container::Style {
        text_color: Some(TEXT_PRIMARY),
        background: Some(Background::Color(Color::from_rgba(0.078, 0.078, 0.106, 0.95))),
        border: Border {
            color: BORDER_SUBTLE,
            width: 0.0,
            radius: 0.0.into(),
        },
        shadow: Shadow {
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.5),
            offset: Vector::new(0.0, 2.0),
            blur_radius: 16.0,
        },
        snap: false,
    }
}

pub fn app_background(_theme: &iced::Theme) -> container::Style {
    container::Style {
        text_color: Some(TEXT_PRIMARY),
        background: Some(Background::Color(BG_DEEP)),
        border: border_none(),
        shadow: shadow_none(),
        snap: false,
    }
}

pub fn preview_container(_theme: &iced::Theme) -> container::Style {
    container::Style {
        text_color: Some(TEXT_MUTED),
        background: Some(Background::Color(BG_ELEVATED)),
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: R_MD.into(),
        },
        shadow: shadow_none(),
        snap: false,
    }
}

// ── Custom Text Input Style ─────────────────────────────────────────────────

pub fn text_input_style(_theme: &iced::Theme, status: text_input::Status) -> text_input::Style {
    let active = text_input::Style {
        background: Background::Color(BG_DEEP),
        border: Border { color: ACCENT, width: 1.5, radius: R_MD.into() },
        icon: TEXT_SECONDARY,
        placeholder: TEXT_MUTED,
        value: TEXT_PRIMARY,
        selection: Color::from_rgba(0.506, 0.227, 0.929, 0.3),
    };

    let hovered = text_input::Style {
        border: border_glow(R_MD),
        ..active
    };

    let focused = text_input::Style {
        border: Border { color: ACCENT_HOVER, width: 2.0, radius: R_MD.into() },
        ..active
    };

    let disabled = text_input::Style {
        background: Background::Color(BG_SURFACE),
        border: border_subtle(R_MD),
        icon: TEXT_MUTED,
        placeholder: TEXT_MUTED,
        value: TEXT_MUTED,
        selection: Color::TRANSPARENT,
    };

    match status {
        text_input::Status::Active => active,
        text_input::Status::Hovered => hovered,
        text_input::Status::Focused { .. } => focused,
        text_input::Status::Disabled => disabled,
    }
}

// ── Custom Scrollable Style ─────────────────────────────────────────────────

pub fn scrollable_style(_theme: &iced::Theme, _status: scrollable::Status) -> scrollable::Style {
    scrollable::Style {
        container: container::Style {
            text_color: None,
            background: None,
            border: border_none(),
            shadow: shadow_none(),
            snap: false,
        },
        vertical_rail: scrollable::Rail {
            background: Some(Background::Color(Color::TRANSPARENT)),
            border: border_none(),
            scroller: scrollable::Scroller {
                background: Background::Color(Color::from_rgba(0.506, 0.227, 0.929, 0.45)),
                border: Border { color: Color::TRANSPARENT, width: 0.0, radius: R_FULL.into() },
            },
        },
        horizontal_rail: scrollable::Rail {
            background: Some(Background::Color(Color::TRANSPARENT)),
            border: border_none(),
            scroller: scrollable::Scroller {
                background: Background::Color(Color::TRANSPARENT),
                border: border_none(),
            },
        },
        gap: None,
        auto_scroll: scrollable::AutoScroll {
            background: Background::Color(Color::TRANSPARENT),
            border: border_none(),
            shadow: shadow_none(),
            icon: Color::TRANSPARENT,
        },
    }
}

// ── Status dot ──────────────────────────────────────────────────────────────

pub fn status_dot(color: Color) -> impl Fn(&iced::Theme) -> container::Style {
    move |_theme: &iced::Theme| container::Style {
        text_color: None,
        background: Some(Background::Color(color)),
        border: Border { color: Color::TRANSPARENT, width: 0.0, radius: R_FULL.into() },
        shadow: Shadow { color: Color::from_rgba(0.0, 0.0, 0.0, 0.3), offset: Vector::new(0.0, 0.0), blur_radius: 4.0 },
        snap: false,
    }
}


