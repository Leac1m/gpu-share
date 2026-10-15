// How the host screens talk about the hardware: a graphics card, or (on a Linux host without
// one) the computer's processor and memory.
import type { HostView } from './types';

export function hardware(h: HostView) {
	const card = !!h.card;
	return {
		card,
		/** "graphics card" / "computer" */
		thing: card ? 'graphics card' : 'computer',
		/** "card" / "computer" */
		short: card ? 'card' : 'computer',
		/** where a loaded model lives: "onto your card" / "into memory" */
		onto: card ? 'onto your card' : 'into memory',
		memoryLabel: card ? 'Graphics card memory' : 'Memory',
		/** total memory a model can use, in GB */
		totalGB: h.card?.vramGB ?? h.memoryGB ?? 8,
		/** room left for everything else: a little on a card, more when it's the system's RAM */
		reserveGB: card ? 0.4 : 2,
		shareLabel: card ? 'Share my graphics card' : 'Share this computer'
	};
}
