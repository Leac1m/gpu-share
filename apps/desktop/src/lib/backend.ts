// Which backend drives the screens: the real one inside the Tauri app, the mock in the
// browser preview. Both implement the interfaces in ./types.
import * as mock from './mock/backend.svelte';
import * as real from './real/backend.svelte';

/** True in the browser preview, where time runs faster and the test bench fakes real life. */
export const isPreview = !(typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window);

export const hostBackend = isPreview ? mock.hostBackend : real.hostBackend;
export const clientBackend = isPreview ? mock.clientBackend : real.clientBackend;
