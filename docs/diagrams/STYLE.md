# Diagram style guide

All diagrams in `docs/diagrams/` are draw.io files that follow this guide so the set reads as one system.
Edit them with draw.io (desktop or diagrams.net); keep the source `.drawio` uncompressed XML.

## Canvas

- White page background (`background="#FFFFFF"`), grid on while editing but `grid="0"` saved for export, page view off.
- Generous margins: nothing closer than 40 px to the content bounds; 60–80 px between unrelated groups.
- **Title block** top-left: diagram name (20 px bold), level/type on the line below (e.g. "C4 · Level 2 — Containers",
  "UML · Sequence diagram", 12 px muted), and a one-line description (11 px muted).
- **Legend** bottom-left: one sample of every element kind and line style used on the diagram.
- **Footer** bottom-right: `IMT · updated 2026-10-18` (10 px muted). Update the date when the diagram changes.

## Typography

- Font: `fontFamily=IBM Plex Sans` plus `fontSource=` the URL-encoded Google Fonts CSS
  (`https://fonts.googleapis.com/css2?family=IBM+Plex+Sans:ital,wght@0,400;0,700;1,400&display=block`) on every cell, so
  draw.io and the viewer load it; without it the renderer falls back to Helvetica.
- Title 20 px bold. Element names 13–14 px bold. Technology line 11 px italic in the muted colour, in square brackets
  (`[Rust crate]`). Descriptions 11 px.
- Labels are HTML (`html=1;whiteSpace=wrap`) so text wraps inside the shape. Text never overflows a shape: size the
  shape to the text and check every shape in a rendered preview.

## Palette

The app's colour-blind-safe series tokens (light theme). Fills are ~12–15 % tints of the stroke colour.

| Role                                   | Stroke    | Fill      |
| -------------------------------------- | --------- | --------- |
| IMT containers / components (primary)  | `#3869BB` | `#E3EAF6` |
| People, actors and the Angular UI      | `#E65F2A` | `#FCE9E1` |
| Rust engine / core                     | `#29A077` | `#E0F2EC` |
| Plugins and extensions                 | `#A664FE` | `#F2E8FF` |
| External systems (neutral)             | `#8A93A6` | `#EEF0F3` |

- Text `#1B2130`; muted text `#5A6375`; edges `#5A6375`.
- Strokes 1.5 px. No gradients. No shadows, except a subtle shadow on top-level containers / system boundaries.

## Shapes

- Rounded rectangles: `rounded=1;arcSize=8;absoluteArcSize=1` (arc size in px).
- C4 notation:
  - **Person** — orange; head-and-body shape (`shape=umlActor` for UML, a rounded box with a person glyph header for C4).
  - **Software system / container / component** — rounded rectangle coloured by role (table above).
  - **External** element — neutral palette with a dashed border (`dashed=1;dashPattern=6 4`).
  - **Planned** element (not built yet) — its role colour, dashed border, white fill and `(planned)` after the name.
  - **System boundary** — dashed rounded group (`dashed=1;fillColor=none`), label top-left, 12 px bold muted.
- Every element label has three lines: **Name** (bold) / *[Technology]* (italic muted) / one-line description.
- Database / store: `shape=cylinder3`, same palette rules.

## Relationships

- Orthogonal edges with rounded corners: `edgeStyle=orthogonalEdgeStyle;rounded=1;strokeWidth=1.5;strokeColor=#5A6375`.
- Arrowheads `endArrow=block;endFill=1;endSize=6`. Planned/optional flows are dashed.
- Labels are verb + protocol, e.g. "Invokes commands / Tauri IPC", "Streams Arrow batches / gRPC over named pipe",
  written on two lines: the verb phrase, then the protocol as the italic muted technology line (`[Tauri IPC]`).
  Edge labels do not wrap (no `whiteSpace=wrap` on edges). Label background white (`labelBackgroundColor=#FFFFFF`), 11 px.
- "Implements" (realisation of a port or interface) is dashed with a hollow block arrowhead.
- Pin edge ends with `exitX/exitY` and `entryX/entryY` so that edges between aligned shapes are straight.
- Avoid crossing lines; route around shapes with waypoints where needed.
- Align everything to a 10 px grid with even spacing.

## UML

- Standard notation per diagram type, same palette and typography:
  - Use cases: actors orange stick figures (name beside the figure, on the side away from the system), system
    boundary dashed rounded rectangle, use cases as primary-tinted ellipses, associations as plain straight lines
    ending at the ellipse's near tip, `«include»`/`«extend»` as dashed open-arrow edges, generalisation with a hollow
    triangle.
  - Sequence: participant heads coloured by role, lifelines neutral dashed (`#8A93A6`), activation bars primary
    (`#3869BB` stroke, `#E3EAF6` fill); synchronous calls solid with filled arrowheads, replies dashed with open
    arrowheads.
  - Class: classes as swimlane boxes coloured by role; inheritance hollow triangle, realisation dashed hollow triangle,
    composition filled diamond, dependency dashed open arrow.
  - State / activity: rounded states in primary tint; initial node filled `#1B2130` circle; final node bullseye;
    decisions as diamonds; guards in `[brackets]`.
  - Deployment: nodes as 3-D boxes (`shape=cube`) in neutral, artifacts/execution environments inside coloured by role.
- Arrowheads are consistent within a diagram type across the set.

## Verifying

Parse each file as XML and render it (draw.io desktop CLI `draw.io -x -f png`, or the official viewer
`viewer-static.min.js` in a headless browser) and inspect the PNG for overlaps, overflowing text and alignment
before committing.
