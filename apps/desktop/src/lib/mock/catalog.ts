// The fake model catalog and VRAM math, ported verbatim from docs/gpu-share-preview.html.
import type { CatalogEntry } from '../types';

export const CARD = { vram: 8.0, name: 'RTX A2000 8 GB' };

export const CATALOG: CatalogEntry[] = [
	{
		id: 'llama32-3b',
		name: 'Llama 3.2 3B Instruct',
		quant: 'Q4_K_M',
		fileGB: 2.0,
		kvGB: 0.25,
		tps: 55,
		desc: 'Quick and light. Good for short questions, rewriting and summaries.',
		license: 'Llama 3.2 Community License'
	},
	{
		id: 'mistral7b',
		name: 'Mistral 7B Instruct v0.3',
		quant: 'Q4_K_M',
		fileGB: 4.4,
		kvGB: 0.5,
		tps: 31,
		desc: 'Steady all-rounder for everyday chat.',
		license: 'Apache 2.0'
	},
	{
		id: 'qwen25-7b',
		name: 'Qwen2.5 7B Instruct',
		quant: 'Q4_K_M',
		fileGB: 4.7,
		kvGB: 0.5,
		tps: 28,
		desc: 'The strongest model that fits this card. Better at code and step-by-step answers.',
		license: 'Apache 2.0'
	},
	{
		id: 'qwen25-14b',
		name: 'Qwen2.5 14B Instruct',
		quant: 'Q4_K_M',
		fileGB: 8.9,
		kvGB: 0.9,
		tps: 0,
		desc: 'Noticeably sharper, but it needs a bigger graphics card than this one.',
		license: 'Apache 2.0'
	}
];

export function model(id: string | null | undefined): CatalogEntry | undefined {
	return CATALOG.find((m) => m.id === id);
}

export { vramFor } from '../format';
