# Theme packs

Every `.json` file in this directory is bundled as a built-in UI pack. They are
intentionally ordinary data files: copy one, change a token, and it is a new
pack. The full schema, layout section ids and community workflow live in
[`docs/UI_CUSTOMIZATION.md`](../../docs/UI_CUSTOMIZATION.md).

To make a pack available without rebuilding the app, copy it into the user theme
folder (Settings → Appearance → **open themes folder**) and press **reload**.

Built-ins:

| File | Effect |
|------|--------|
| `instrument.json` | the default warm charcoal / vermilion bench instrument |
| `midnight.json` | cool navy with a signal-blue accent |
| `paper.json` | light paper surfaces with a rust-red accent |
| `focus.json` | compact rows, no motion, calendar/activity/models/status hidden |

Do not put JavaScript, CSS or images here. A pack is tokens only.
