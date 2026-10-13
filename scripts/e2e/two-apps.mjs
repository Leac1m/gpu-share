// Two real GPU Share apps on one machine: an owner sharing with the dev engine, and a friend.
// The friend finds the owner by multicast, pairs with a matching code, and chats with the
// real model. Run through scripts/e2e/run.sh, which starts Xvfb, the UI dev server and two
// tauri-drivers.

import { btn, Session } from './wd.mjs';

const APP = process.env.APP;
const OUT = process.env.OUT ?? '.';
const log = (...a) => console.log('[e2e]', ...a);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const owner = await Session.start('http://127.0.0.1:4444', APP, 'owner');
const friend = await Session.start('http://127.0.0.1:4454', APP, 'friend');
try {
	// --- the owner switches sharing on
	await owner.click(btn('Set up sharing'));
	await owner.click('//div[contains(@class,"bigswitch")]//button[@role="switch"]');
	await owner.wait('//div[contains(@class,"bigswitch")]//span[contains(@class,"chip") and contains(., "Ready")]');
	log('owner: sharing, Ready');

	// --- the friend finds the owner's computer
	await friend.click(btn("Use a friend's"));
	await friend.wait('//div[contains(@class,"device")]//div[@class="nm" and contains(., "E2E Owner")]', 30000);
	log('friend: owner found on the network');
	await friend.click(btn('Ask to connect'));
	const friendCode = (await friend.text('//div[contains(@class,"banner info")]//span[contains(@class,"mono")]')).trim();
	const ownerCode = (await owner.text('//div[contains(@class,"modal")]//div[contains(@class,"code")]')).trim();
	log('codes', { friendCode, ownerCode });
	if (friendCode !== ownerCode || !/^\d{6}$/.test(ownerCode)) throw new Error('pairing codes differ');
	const popup = await owner.text('//div[contains(@class,"modal")]//h3');
	log('owner popup:', popup);
	await owner.screenshot(`${OUT}/e2e-owner-popup.png`);
	await owner.click(btn('Allow'));

	// --- paired: the friend lands in the chat and asks something
	await friend.wait('//div[contains(@class,"chathead")]//select/option', 30000);
	log('friend: chat open, model', await friend.text('//div[contains(@class,"chathead")]//select/option'));
	await friend.type('//div[contains(@class,"composer")]//input', 'What is the capital of France?');
	await friend.click(btn('Send'));
	await friend.wait('//div[contains(@class,"msg ai")]', 90000);
	// wait for the reply to finish (the caret goes away)
	const end = Date.now() + 90000;
	while (Date.now() < end) {
		const streaming = await friend.js('return !!document.querySelector(".msg.ai .caret")');
		if (!streaming) break;
		await sleep(300);
	}
	const reply = await friend.text('(//div[contains(@class,"msg ai")])[last()]');
	log('friend: reply:', JSON.stringify(reply));
	if (!reply.trim()) throw new Error('empty reply');
	await friend.screenshot(`${OUT}/e2e-friend-chat.png`);

	// --- the owner sees usage, never the text
	await owner.wait('//div[@class="d" and contains(., "1 question answered")]', 15000);
	const ownerText = await owner.js('return document.body.innerText');
	if (ownerText.includes('capital of France')) throw new Error('the owner can see the question!');
	log('owner: 1 question counted, question text not shown');

	// --- the owner pauses by switching off: the friend sees the host disappear
	await owner.click('//div[contains(@class,"bigswitch")]//button[@role="switch"]');
	await friend.click('//div[@role="tablist"]//button[contains(., "Computers")]');
	await friend.wait('//div[contains(@class,"big") and contains(., "Looking for computers")]', 30000);
	log('friend: owner disappeared after sharing was switched off');
	await owner.screenshot(`${OUT}/e2e-owner-share.png`);
	log('PASS');
} finally {
	await owner.end();
	await friend.end();
}
