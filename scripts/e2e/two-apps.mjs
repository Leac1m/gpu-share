// Two real GPU Share apps on one machine: an owner sharing with the dev engine, and a friend.
// Both run Linux and the owner has no graphics card (the world file lists none), the way an
// Ubuntu-to-Ubuntu pilot without GPUs looks: the owner answers on the processor.
// The friend finds the owner by multicast, pairs with a matching code, and chats with the
// real model. Run through scripts/e2e/run.sh, which starts Xvfb, the UI dev server and two
// tauri-drivers.

import { writeFileSync } from 'node:fs';
import { btn, Session } from './wd.mjs';

const tab = (label) => `//div[contains(@class,"tabs")]//button[normalize-space(.)="${label}"]`;

const APP = process.env.APP;
const OUT = process.env.OUT ?? '.';
const log = (...a) => console.log('[e2e]', ...a);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const owner = await Session.start('http://127.0.0.1:4444', APP, 'owner');
const friend = await Session.start('http://127.0.0.1:4454', APP, 'friend');
try {
	// --- the owner switches sharing on. Without a graphics card the app opens on "Use a
	// friend's", so the owner goes to Share mode first.
	await owner.click('//header//button[contains(., "Share this computer")]');
	await owner.click(btn('Set up sharing'));
	await owner.wait('//div[contains(@class,"banner")]//b[contains(., "No graphics card found")]');
	await owner.click('//div[contains(@class,"bigswitch")]//button[@role="switch"]');
	await owner.wait('//div[contains(@class,"bigswitch")]//span[contains(@class,"chip") and contains(., "Ready")]');
	log('owner: sharing, Ready');

	// --- the friend finds the owner's computer
	await friend.click('//header//button[normalize-space(.)="Chat"]');
	await friend.wait('//div[contains(@class,"device")]//div[@class="nm" and contains(., "E2E Owner")]', 30000);
	log('friend: owner found on the network');
	const seen = await friend.text('//div[contains(@class,"device")][.//div[contains(., "E2E Owner")]]//div[@class="ds"]');
	if (!seen.includes('no graphics card')) throw new Error(`friend should be told the owner has no card: ${seen}`);
	log('friend: sees', JSON.stringify(seen.trim()));
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

	// --- the model library: remove the model, add it back from the catalog (real download)
	await owner.click(tab('Models'));
	const devCard = '//div[contains(@class,"model")][.//div[contains(@class,"nm") and contains(., "SmolLM2")]]';
	await owner.wait(`${devCard}//span[contains(., "Fits your computer")]`);
	await owner.click(`${devCard}//button[contains(., "Remove")]`);
	await owner.click(`${devCard}//button[normalize-space(.)="Add"]`, 15000);
	log('owner: removed the model, downloading it again…');
	await owner.wait(`//div[contains(@class,"toast") and contains(., "Model added")]`, 180000);
	log('owner: "Model added" (downloaded and checksum verified)');
	await owner.click(tab('Share'));
	await friend.type('//div[contains(@class,"composer")]//input', 'Say hello.');
	await friend.click(btn('Send'));
	await friend.wait('(//div[contains(@class,"msg ai")])[2]', 90000);
	log('friend: answered by the re-downloaded model');

	// --- a pause rule: Blender opens on the owner's computer (via the dev world file)
	writeFileSync(`${OUT}/world.json`, JSON.stringify({ running_apps: ['Blender'] }));
	await owner.wait('//div[contains(@class,"banner warn") and contains(., "Blender is open")]', 15000);
	log('owner: paused, "Blender is open"');
	const pausedBanner = await friend.text('//div[contains(@class,"msgs")]//div[contains(@class,"banner warn")]', 20000);
	log('friend sees:', JSON.stringify(pausedBanner));
	if (pausedBanner.includes('Blender')) throw new Error('the friend was told which program is open!');
	if (!pausedBanner.includes("They're using their computer")) throw new Error('no general reason shown');
	await friend.screenshot(`${OUT}/e2e-friend-paused.png`);
	writeFileSync(`${OUT}/world.json`, JSON.stringify({}));
	await friend.wait('//div[contains(@class,"chathead")]//span[contains(@class,"chip") and contains(., "Ready")]', 20000);
	log('friend: Ready again once Blender closed');

	// --- the owner switches sharing off: the friend sees the host disappear
	await owner.click('//div[contains(@class,"bigswitch")]//button[@role="switch"]');
	await friend.click(tab('Computers'));
	await friend.wait('//div[contains(@class,"big") and contains(., "Looking for computers")]', 30000);
	log('friend: owner disappeared after sharing was switched off');

	// --- the owner uses his own GPU himself, with sharing off (P9-T1)
	await owner.click('//header//button[normalize-space(.)="Chat"]');
	await owner.click('//div[contains(@class,"device")][.//div[@class="nm" and normalize-space(.)="This computer"]]//button[normalize-space(.)="Chat"]', 15000);
	await owner.wait('//div[contains(@class,"chathead")]//select/option', 30000);
	await owner.type('//div[contains(@class,"composer")]//input', 'Say hi to the owner.');
	await owner.click(btn('Send'));
	await owner.wait('//div[contains(@class,"msg ai")]', 90000);
	const ownEnd = Date.now() + 90000;
	while (Date.now() < ownEnd && (await owner.js('return !!document.querySelector(".msg.ai .caret")'))) await sleep(300);
	const ownReply = await owner.text('(//div[contains(@class,"msg ai")])[last()]');
	if (!ownReply.trim()) throw new Error('the owner got no answer from his own computer');
	log('owner: answered by his own computer with sharing off:', JSON.stringify(ownReply.slice(0, 60)));
	await owner.screenshot(`${OUT}/e2e-owner-own-chat.png`);
	await owner.screenshot(`${OUT}/e2e-owner-share.png`);
	log('PASS');
} catch (e) {
	await owner.screenshot(`${OUT}/fail-owner.png`).catch(() => {});
	await friend.screenshot(`${OUT}/fail-friend.png`).catch(() => {});
	throw e;
} finally {
	await owner.end();
	await friend.end();
}
