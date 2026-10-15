// A deliberately small markdown renderer for chat replies.
//
// Replies come from another machine, and this HTML is inserted into a webview that can call
// Tauri commands, so everything is escaped first and only a fixed set of tags is produced.
// The message bubble keeps `white-space: pre-wrap`, so line breaks and list layout survive
// without paragraph tags.

const ESC: Record<string, string> = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' };
export const escapeHtml = (s: string) => s.replace(/[&<>"']/g, (c) => ESC[c]);

function inline(s: string): string {
	// s is already escaped; split out inline code first so nothing inside it is formatted
	return s
		.split(/(`[^`\n]+`)/)
		.map((part) =>
			part.startsWith('`') && part.endsWith('`') && part.length > 1
				? '<code>' + part.slice(1, -1) + '</code>'
				: part
						.replace(/\*\*([^*\n]+)\*\*/g, '<b>$1</b>')
						.replace(/(^|[^*\w])\*([^*\s](?:[^*\n]*[^*\s])?)\*(?!\w)/g, '$1<i>$2</i>')
						.replace(/^(#{1,6}) +(.+)$/gm, '<b>$2</b>')
		)
		.join('');
}

export function renderMarkdown(text: string): string {
	const out: string[] = [];
	// fenced code blocks; an unclosed fence (still streaming) runs to the end
	const re = /```[^\n]*\n?([\s\S]*?)(?:```|$)/g;
	let last = 0;
	for (let m = re.exec(text); m; m = re.exec(text)) {
		if (m[0] === '') break;
		out.push(inline(escapeHtml(text.slice(last, m.index))));
		out.push('<pre><code>' + escapeHtml(m[1].replace(/\n$/, '')) + '</code></pre>');
		last = m.index + m[0].length;
	}
	out.push(inline(escapeHtml(text.slice(last))));
	return out.join('');
}
