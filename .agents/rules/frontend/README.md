# Frontend coding rules

These rules apply to Customer Admin, Customer Member, the Customer demo, Operations Console, Website, and shared frontend UI. Repository workflow and verification rules remain in the root `AGENTS.md`.

Read [core.md](core.md) for every frontend task. Then read the files relevant to the change:

| File | When to read |
| --- | --- |
| [components-and-layout.md](components-and-layout.md) | Page structure, navigation, forms, tables, upload controls, and shared components |
| [typography.md](typography.md) | Font family, the four-size type scale, tables, lists, buttons, and form controls |
| [interaction-and-copy.md](interaction-and-copy.md) | Disabled actions, statuses, feedback, help text, and localization |
| [responsive-and-media.md](responsive-and-media.md) | Responsive layout, scrolling, login and hero illustrations |
| [code-surfaces.md](code-surfaces.md) | Copyable commands, tokens, passwords, and code blocks |

Use the existing implementation and shared UI package as the visual source of truth. If a rule needs to change, update its topic file and keep the root entry point valid; do not create a competing page-level rule copy.
