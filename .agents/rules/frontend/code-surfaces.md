# Code surfaces and copy actions

- Use the shared `ACopyCode` component for copyable single-line commands, passwords, tokens, and ordinary multiline code surfaces instead of recreating page-local copy-button structures or styles.
- Code-block actions such as Copy must be anchored above the scrollable code content, normally at the top-right corner, and must not move with or consume space inside the scrolling layer.
- In VitePress documentation, retain the native code rendering and copy controls. Code backgrounds, plain text, syntax highlighting and custom model examples must follow the active light/dark theme; do not force dark highlighting into both themes.
- A single-line command field is the exception: keep its command and action as normal-flow siblings in one Flex or Grid row, vertically center both, and preserve the dark code-surface treatment during hover instead of turning the action into a detached light button.
