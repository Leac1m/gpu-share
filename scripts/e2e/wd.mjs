// A tiny WebDriver client (enough for tauri-driver), so the e2e check needs no npm packages.

const EL = 'element-6066-11e4-a52e-4f735466cecf';

export class Session {
	constructor(base, id, name) {
		this.base = base;
		this.id = id;
		this.name = name;
	}

	static async start(base, application, name) {
		const r = await fetch(`${base}/session`, {
			method: 'POST',
			headers: { 'content-type': 'application/json' },
			body: JSON.stringify({ capabilities: { alwaysMatch: { 'tauri:options': { application } } } })
		});
		const j = await r.json();
		if (!j.value?.sessionId) throw new Error(`${name}: no session: ${JSON.stringify(j)}`);
		return new Session(base, j.value.sessionId, name);
	}

	async cmd(method, path, body) {
		const r = await fetch(`${this.base}/session/${this.id}${path}`, {
			method,
			headers: { 'content-type': 'application/json' },
			body: body === undefined ? undefined : JSON.stringify(body)
		});
		const j = await r.json();
		if (j.value && j.value.error) throw new Error(`${this.name} ${path}: ${j.value.error}: ${j.value.message}`);
		return j.value;
	}

	async find(xpath) {
		const v = await this.cmd('POST', '/element', { using: 'xpath', value: xpath });
		const id = v && (v[EL] ?? v.ELEMENT);
		if (!id) throw new Error(`no element for ${xpath}: ${JSON.stringify(v)}`);
		return id;
	}

	/** Waits until `xpath` exists, then returns it. */
	async wait(xpath, timeoutMs = 20000) {
		const end = Date.now() + timeoutMs;
		let last;
		while (Date.now() < end) {
			try {
				return await this.find(xpath);
			} catch (e) {
				last = e;
				await new Promise((r) => setTimeout(r, 200));
			}
		}
		throw new Error(`${this.name}: timed out waiting for ${xpath} (${last?.message})`);
	}

	async click(xpath, timeoutMs) {
		const el = await this.wait(xpath, timeoutMs);
		await this.cmd('POST', `/element/${el}/click`, {});
	}

	async type(xpath, text) {
		const el = await this.wait(xpath);
		await this.cmd('POST', `/element/${el}/value`, { text });
	}

	async text(xpath, timeoutMs) {
		const el = await this.wait(xpath, timeoutMs);
		return this.cmd('GET', `/element/${el}/text`);
	}

	async js(script, args = []) {
		return this.cmd('POST', '/execute/sync', { script, args });
	}

	async screenshot(path) {
		const b64 = await this.cmd('GET', '/screenshot');
		const fs = await import('node:fs');
		fs.writeFileSync(path, Buffer.from(b64, 'base64'));
	}

	async end() {
		await fetch(`${this.base}/session/${this.id}`, { method: 'DELETE' }).catch(() => {});
	}
}

export const btn = (label) => `//button[contains(normalize-space(.), "${label}")]`;
