// Settings layout constants, kept apart from the kit so the window shell can
// draw a settings-shaped placeholder before the lazy settings chunk arrives.

/** The settings content column: centered in the space beside the rail, 46rem
 *  of content at most. Every page and the loading skeleton use it, so content
 *  never jumps as the rail selection changes, and a wide window gains even
 *  margins on both sides instead of one dead strip on the right. */
export const SETTINGS_COLUMN = "mx-auto w-full max-w-200 px-6 pb-12 pt-7 md:px-10";

/** The wide variant for a page that docks a second column (Storage's chat). */
export const SETTINGS_COLUMN_WIDE = "mx-auto w-full max-w-page-wide px-6 pb-16 pt-7 md:px-10";
