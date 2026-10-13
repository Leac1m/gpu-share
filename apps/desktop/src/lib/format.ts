// Small display helpers shared by the screens and the mock.

/** Tokens are what the engine counts; words are what people understand. */
export const WORDS_PER_TOKEN = 0.75;

/** "about 750" style approximation of a token count in words. */
export function approxWords(tokens: number): string {
	const w = tokens * WORDS_PER_TOKEN,
		step = w < 1000 ? 50 : 500;
	return (Math.round(w / step) * step).toLocaleString('en-US');
}

export const toTokens = (words: number) => Math.round(words / WORDS_PER_TOKEN);
export const fmtNum = (n: number) => n.toLocaleString('en-US');

export function ordinal(n: number): string {
	const s = ['th', 'st', 'nd', 'rd'];
	return n + (s[((n % 100) - 20) % 10] || s[n % 100] || 'th');
}

export function fmtHour(h: number): string {
	return (h % 12 === 0 ? 12 : h % 12) + (h < 12 ? 'am' : 'pm');
}

export const minutes = (sec: number) => Math.round((sec / 60) * 10) / 10;

/** VRAM estimate used for the "fits your card" badge (spec §6.5). */
export function vramFor(m: { fileGB: number; kvGB: number }, ctx: number): number {
	return m.fileGB + m.kvGB * (ctx / 4096) + 0.5;
}

/** Offered in "Add a program…" next to whatever is running now (roadmap D6, not yet confirmed). */
export const SUGGESTED_PAUSE_APPS = [
	'Blender',
	'DaVinci Resolve',
	'Photoshop',
	'Premiere Pro',
	'After Effects',
	'Unreal Engine',
	'Cinema 4D',
	'Steam',
	'OBS Studio'
];
