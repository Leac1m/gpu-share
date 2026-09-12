// Canned chat replies, ported verbatim from docs/gpu-share-preview.html.

interface ReplyRule {
	k: string[];
	t: string;
}

const REPLIES: ReplyRule[] = [
	{
		k: ['hello', 'hi', 'hey'],
		t: "Hello. I'm running on Dara's RTX A2000 over your home network. Ask me anything you'd normally ask a chat model."
	},
	{
		k: ['gpu', 'graphics', 'vram', 'card'],
		t: "The A2000 has 8 GB of memory, which comfortably holds a 7B model at 4-bit along with the conversation. A 14B model would need roughly 10 GB, so it won't fit on this card. Splitting one model across two computers is possible but slow over a home network, which is why this setup keeps each model on a single machine."
	},
	{
		k: ['recipe', 'cook', 'dinner'],
		t: 'A quick one: fry chopped onion and garlic in oil until soft, add a tin of tomatoes and a pinch of sugar, simmer fifteen minutes, then stir through cooked pasta with plenty of black pepper and whatever cheese you have. Works on a weeknight and uses one pan.'
	},
	{
		k: ['python', 'code', 'script', 'function'],
		t: "Here's a small example:\n\ndef chunks(items, size):\n    for i in range(0, len(items), size):\n        yield items[i:i + size]\n\nIt splits a list into pieces of a fixed size and yields them one at a time, so you never hold more than one chunk in memory. Call it with list(chunks(rows, 100))."
	},
	{
		k: ['email', 'write', 'draft'],
		t: "Try this:\n\nHi Sam,\n\nThanks for sending the files over. I've had a look and the second version works better for what we discussed. Could you send the source file when you get a moment?\n\nBest,\nAlex\n\nShort, says what you want, and gives them one clear thing to do."
	},
	{
		k: ['explain', 'what is', 'how does'],
		t: 'The short version: the model is a large set of numbers that predicts the next piece of text, one piece at a time, based on everything before it. Nothing is looked up and nothing is remembered between conversations. What makes it feel coherent is that each prediction takes the whole conversation so far into account.'
	}
];

const FALLBACK = [
	"That depends on what you're after. If you tell me a bit more about the situation, I can give you something more specific than a general answer.",
	"Good question. There isn't one right answer, but the usual approach is to start with the simplest version that works, see where it breaks, and only add complexity when something actually forces you to.",
	"I'd break that into three parts: what you already know, what you need to find out, and what you can safely ignore for now. Most of the difficulty usually sits in the middle part."
];

export function replyFor(q: string): string {
	const low = q.toLowerCase();
	const hit = REPLIES.find((r) => r.k.some((k) => low.includes(k)));
	return hit ? hit.t : FALLBACK[Math.floor(Math.random() * FALLBACK.length)];
}
