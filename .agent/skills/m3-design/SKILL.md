---
name: m3-design
description: >-
  Provides comprehensive Material Design 3 (Material You / M3) guidelines, design tokens, color systems,
  typography scales, elevation, shape hierarchy, and component patterns. Use when designing, styling,
  or refactoring UI components, web interfaces, extensions, or desktop applications with Material 3 principles.
---

# Material Design 3 (M3) Skill

This skill provides direct guidance and reference tokens for implementing Google's **Material Design 3 (M3 / Material You)** specification across desktop apps (egui/eframe), browser extensions, and web UI.

---

## 1. Core Principles of M3

1. **Adaptive & Personal**: Dynamic color and expressive tonal contrast that adapts to theme modes (Light & Dark).
2. **Accessible & Ergonomic**: Minimum 4.5:1 contrast for normal text and 3:1 for large text/icons; clear state layers for hover, focus, and pressed states.
3. **Layered Elevation**: Elevation expressed via surface color tones (`surface-container-*`) in addition to subtle shadows.

---

## 2. Color System & Role Hierarchy

M3 organizes color into semantic roles rather than raw hex values. Each role exists in both Light and Dark themes.

### Key Roles
- **Primary / On Primary**: High-emphasis elements, prominent buttons, active navigation.
- **Primary Container / On Primary Container**: Medium-emphasis fills, fab containers, key card accents.
- **Secondary / On Secondary**: Less prominent components (filter chips, secondary actions).
- **Secondary Container / On Secondary Container**: Selected state for segmented buttons, navigation indicator.
- **Tertiary / On Tertiary**: Contrasting accent for balance, playful highlights, badge counters.
- **Surface / On Surface**: Neutral background and base layer for views, windows, and reading text.
- **Surface Container (Lowest, Low, Default, High, Highest)**: Stepped surface containers to create depth without relying solely on drop shadows.
- **Outline / Outline Variant**: Subtle decorative dividers, borders, and input boundaries.
- **Error / On Error / Error Container**: Critical state and destructive actions.

### Surface Elevation Levels (Dark Theme Default Palette Reference)
| Level | Surface Token | Opacity / Tint | Typical Usage |
|---|---|---|---|
| Level 0 | `surface` | Base bg (`#111318`) | Window canvas, background |
| Level 1 | `surface-container-low` | Subtle tint (`#191C20`) | Default card background |
| Level 2 | `surface-container` | Standard tint (`#1D2024`) | Content sections, sidebars |
| Level 3 | `surface-container-high` | Elevated (`#282A2F`) | Menus, dropdowns, floating panels |
| Level 4 | `surface-container-highest` | High tint (`#33353A`) | Dialogs, modals, floating action bars |

---

## 3. Typography Scale

M3 defines 15 typography styles grouped into 5 scale categories:

| Category | Role | Size | Line Height | Weight | Tracking |
|---|---|---|---|---|---|
| **Display** | Large | 57px | 64px | Regular (400) | -0.25px |
| | Medium | 45px | 52px | Regular (400) | 0px |
| | Small | 36px | 44px | Regular (400) | 0px |
| **Headline** | Large | 32px | 40px | Regular (400) | 0px |
| | Medium | 28px | 36px | Regular (400) | 0px |
| | Small | 24px | 32px | Regular (400) | 0px |
| **Title** | Large | 22px | 28px | Regular (400) | 0px |
| | Medium | 16px | 24px | Medium (500) | +0.15px |
| | Small | 14px | 20px | Medium (500) | +0.1px |
| **Body** | Large | 16px | 24px | Regular (400) | +0.5px |
| | Medium | 14px | 20px | Regular (400) | +0.25px |
| | Small | 12px | 16px | Regular (400) | +0.4px |
| **Label** | Large | 14px | 20px | Medium (500) | +0.1px |
| | Medium | 12px | 16px | Medium (500) | +0.5px |
| | Small | 11px | 16px | Medium (500) | +0.5px |

---

## 4. Shape & Corner Radii

M3 uses deliberate corner radii to group related information:

* **None**: `0px` (Full-bleed surfaces, strict dividers)
* **Extra Small**: `4px` (Tooltips, compact status badges)
* **Small**: `8px` (Chips, mini buttons, input fields)
* **Medium**: `12px` (Cards, small dialogs, dropdown menus)
* **Large**: `16px` (Standard dialogs, floating action sheets)
* **Extra Large**: `28px` (Large cards, navigation bars, bottom sheets)
* **Full (Pill)**: `9999px` (Pill buttons, segmented controls, search bars)

---

## 5. Interaction & State Layers

State layers apply a semi-transparent overlay of the `on-*` color over the base container:

* **Hover**: 8% opacity (`rgba(..., 0.08)`)
* **Focus**: 12% opacity (`rgba(..., 0.12)`)
* **Pressed**: 12% opacity (`rgba(..., 0.12)`)
* **Dragged**: 16% opacity (`rgba(..., 0.16)`)
* **Disabled**: 38% opacity on text/icons, 12% opacity on container fills

---

## 6. Implementation Patterns

### CSS / Web Tokens (Extensions & Webviews)
```css
:root {
  --md-sys-color-primary: #A8C7FA;
  --md-sys-color-on-primary: #062E6F;
  --md-sys-color-primary-container: #0842A0;
  --md-sys-color-on-primary-container: #D3E3FD;

  --md-sys-color-surface: #111318;
  --md-sys-color-on-surface: #E2E2E9;
  --md-sys-color-surface-container: #1D2024;
  --md-sys-color-surface-container-high: #282A2F;
  --md-sys-color-outline: #8E9099;
  --md-sys-color-outline-variant: #44474E;

  --md-sys-shape-corner-small: 8px;
  --md-sys-shape-corner-medium: 12px;
  --md-sys-shape-corner-large: 16px;
  --md-sys-shape-corner-full: 9999px;
}
```

### Rust / egui Tokens (`crates/app/src/theme/`)
When styling desktop components in Cosmo Downloader:
- Map `surface_container_high` to menu backgrounds and dialog fills.
- Use `Rounding::same(8.0)` or `Rounding::same(12.0)` for card borders and popup menus.
- Apply subtle 1px border stroke using `outline_variant` to define edges against dark canvases.
