// UI-only state: which tab is open, whether the preview's tray menu is showing.
// Nothing here is sent to a backend.

export type HostTab = 'share' | 'limits' | 'models' | 'access';
export type ClientTab = 'devices' | 'chat';

export const ui = $state({
	/** The installed app's top-level mode. */
	mode: 'share' as 'share' | 'use',
	hostTab: 'share' as HostTab,
	clientTab: 'devices' as ClientTab,
	trayOpen: false
});
