import { describe, expect, it } from 'vitest';
import { renderMarkdown } from './markdown';

describe('renderMarkdown', () => {
	it('escapes HTML so a reply cannot inject markup', () => {
		const out = renderMarkdown('<img src=x onerror="alert(1)"> <script>x</script>');
		expect(out).not.toContain('<img');
		expect(out).not.toContain('<script');
		expect(out).toContain('&lt;img src=x onerror=&quot;alert(1)&quot;&gt;');
	});

	it('escapes inside code blocks and inline code too', () => {
		expect(renderMarkdown('```html\n<b>hi</b>\n```')).toBe('<pre><code>&lt;b&gt;hi&lt;/b&gt;</code></pre>');
		expect(renderMarkdown('use `<div>`')).toBe('use <code>&lt;div&gt;</code>');
	});

	it('formats bold, italic and headings', () => {
		expect(renderMarkdown('**a** and *b*')).toBe('<b>a</b> and <i>b</i>');
		expect(renderMarkdown('## Title\ntext')).toBe('<b>Title</b>\ntext');
	});

	it('does not format inside inline code', () => {
		expect(renderMarkdown('`**x**`')).toBe('<code>**x**</code>');
	});

	it('renders a fence that is still streaming', () => {
		expect(renderMarkdown('see:\n```py\nprint(1)')).toBe('see:\n<pre><code>print(1)</code></pre>');
	});

	it('leaves plain text and multiplication alone', () => {
		expect(renderMarkdown('2 * 3 * 4')).toBe('2 * 3 * 4');
		expect(renderMarkdown('line one\n- item')).toBe('line one\n- item');
	});
});
