# Responsive layout, scrolling, and media

- Across every frontend surface, native scrollbars must stay hidden and must never reserve layout space. Scrollable content remains operable by wheel, trackpad, touch, and keyboard. When position feedback is needed, use the shared transient overlay scrollbar that appears only while content is actively scrolling. Its default track is 95% of the scroll frame, centered with 2.5% clearance at each end so it never collides with rounded corners.
- Do not add persistent, hover-only, or layout-consuming scrollbars to individual pages or components.
- Do not use `background-size: cover` for composition-sensitive login or hero illustrations whose complete product scene must remain visible. Keep meaningful objects inside the source image's safe area, fit the full image height on desktop, anchor it toward the illustration side, and validate both short-wide and narrow viewports so important objects and foreground cards are not clipped.
