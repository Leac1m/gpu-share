// Which backend drives the screens. The browser preview always uses the mock; the Tauri app
// swaps in the real one (see `real/`).
export { hostBackend, clientBackend } from './mock/backend.svelte';

/** True in the browser preview, where time runs faster and the test bench fakes real life. */
export const isPreview = true;
