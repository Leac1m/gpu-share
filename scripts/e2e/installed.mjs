// See installed.sh. Drives two installed GPU Share apps over WebDriver.

import { execSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { btn, Session } from './wd.mjs';

const tab = (label) => `//div[contains(@class,"tabs")]//button[normalize-space(.)="${label}"]`;
const APP = process.env.APP;
const OUT = process.env.OUT ?? '.';
const log = (...a) => console.log('[installed]', ...a);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const sh = (cmd) => execSync(cmd, { encoding: 'utf8' }).trim();

const owner = await Session.start('http://127.0.0.1:4464', APP, 'owner');
const friend = await Session.start('http://127.0.0.1:4474', APP, 'friend');
try {
	// --- a computer without a card opens on "Use a friend's"; the owner goes to Share mode
	await owner.click('//header//button[contains(., "Share this computer")]');
	await owner.click(btn('Set up sharing'));
	if (existsSync(`${OUT}/owner/engine`)) throw new Error('the engine was downloaded before sharing was switched on');
	await owner.click('//div[contains(@class,"bigswitch")]//button[@role="switch"]');
	await owner.wait('//div[contains(@class,"banner")]//b[contains(., "Getting the AI engine ready")]', 15000);
	log('owner: switched on, downloading the engine');
	await owner.wait('//div[contains(@class,"toast") and contains(., "Ready to share")]', 300000);
	await owner.wait('//div[contains(@class,"bigswitch")]//span[contains(@class,"chip") and contains(., "Ready")]', 30000);
	log('owner: engine set up:', sh(`ls ${OUT}/owner/engine/*/ | grep -c . `), 'files');

	// --- a small model for the processor
	await owner.click(tab('Models'));
	const card = '//div[contains(@class,"model")][.//div[contains(@class,"nm") and contains(., "Qwen3 0.6B")]]';
	await owner.wait(`${card}//span[contains(., "Fits your computer")]`);
	await owner.click(`${card}//button[normalize-space(.)="Add"]`);
	log('owner: adding Qwen3 0.6B (640 MB)…');
	await owner.wait('//div[contains(@class,"toast") and contains(., "Model added")]', 900000);
	log('owner: Qwen3 0.6B added and verified');
	await owner.click(tab('Share'));
	await owner.screenshot(`${OUT}/owner-share.png`);

	// --- the friend finds, pairs and asks
	await friend.wait('//div[contains(@class,"device")]//div[@class="nm" and contains(., "Ubuntu Owner")]', 60000);
	const sub = await friend.text('//div[contains(@class,"device")][.//div[contains(., "Ubuntu Owner")]]');
	log('friend: sees', JSON.stringify(sub.replace(/\s+/g, ' ')));
	await friend.click(btn('Ask to connect'));
	const friendCode = (await friend.text('//div[contains(@class,"banner info")]//span[contains(@class,"mono")]')).trim();
	const ownerCode = (await owner.text('//div[contains(@class,"modal")]//div[contains(@class,"code")]')).trim();
	if (friendCode !== ownerCode) throw new Error('pairing codes differ');
	await owner.click(btn('Allow'));
	log('paired, code', ownerCode);
	await friend.wait('//div[contains(@class,"chathead")]//select/option', 30000);
	await friend.type('//div[contains(@class,"composer")]//input', 'What is the capital of France? Answer in one sentence.');
	await friend.click(btn('Send'));
	await friend.wait('//div[contains(@class,"msg ai")]', 180000);
	const end = Date.now() + 180000;
	while (Date.now() < end && (await friend.js('return !!document.querySelector(".msg.ai .caret")'))) await sleep(300);
	const reply = await friend.text('(//div[contains(@class,"msg ai")])[last()]');
	log('friend: reply from Qwen3 0.6B on the processor:', JSON.stringify(reply));
	if (!/paris/i.test(reply)) throw new Error('unexpected reply');
	await friend.screenshot(`${OUT}/friend-chat.png`);

	// --- the engine as it really runs
	const pid = sh('pgrep -x llama-server');
	const args = sh(`tr '\\0' ' ' < /proc/${pid}/cmdline`);
	const nice = sh(`ps -o ni= -p ${pid}`);
	log('engine:', args.replace(/--api-key \S+/, '--api-key …'));
	log('engine nice:', nice);
	if (!args.includes('--device none')) throw new Error('expected processor-only placement');
	if (!args.includes('--host 127.0.0.1')) throw new Error('engine must listen on localhost only');
	if (Number(nice) < 10) throw new Error('engine should run at a lower priority');
	if (existsSync(`${OUT}/friend/engine`)) throw new Error('the friend downloaded an engine without sharing');
	log('friend: never shared, never downloaded the engine');

	// --- if the app dies, the engine goes with it
	const parent = sh(`ps -o ppid= -p ${pid}`);
	sh(`kill -9 ${parent}`);
	// dead means gone, or a zombie waiting for PID 1 to collect it
	const dead = () => !existsSync(`/proc/${pid}`) || sh(`ps -o stat= -p ${pid} || true`).startsWith('Z');
	for (let i = 0; i < 50 && !dead(); i++) await sleep(100);
	if (!dead()) throw new Error('the engine outlived the app');
	log('owner app killed: the engine went with it');
	log('PASS');
} catch (e) {
	await owner.screenshot(`${OUT}/fail-owner.png`).catch(() => {});
	await friend.screenshot(`${OUT}/fail-friend.png`).catch(() => {});
	throw e;
} finally {
	await owner.end().catch(() => {});
	await friend.end().catch(() => {});
}
