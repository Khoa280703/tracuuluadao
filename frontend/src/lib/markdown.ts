import { marked } from 'marked';
import DOMPurify from 'dompurify';

marked.setOptions({
  breaks: true,
  gfm: true,
});

const renderer = new marked.Renderer();
renderer.link = ({ href, text }) => {
  if (href.startsWith('javascript:')) return '';
  return `<a href="${href}" target="_blank" rel="noopener noreferrer" class="text-blue-600 dark:text-blue-400">${text}</a>`;
};

marked.use({ renderer });

export function sanitizeMarkdown(md: string): string {
  const html = marked.parse(md) as string;
  return DOMPurify.sanitize(html, { ADD_ATTR: ['target', 'rel', 'class'] }) as string;
}

export function sanitizeReportMarkdown(md: string): string {
  const html = marked.parse(formatReportMarkdown(md)) as string;
  return DOMPurify.sanitize(html, { ADD_ATTR: ['target', 'rel', 'class'] }) as string;
}

function formatReportMarkdown(md: string): string {
  let formatted = md.trim();
  const preservedSegments: string[] = [];

  formatted = formatted.replace(/```[\s\S]*?```|`[^`\n]+`/g, (segment) => {
    const token = `__REPORT_SEGMENT_${preservedSegments.length}__`;
    preservedSegments.push(segment);
    return token;
  });

  formatted = formatted.replace(/^\s*Khuyến nghị\s*$/gim, '## Khuyến nghị');
  formatted = formatted.replace(/^\s*Tóm lại,?\s*/gim, '## Kết luận\n');

  formatted = formatted.replace(
    /(^## Khuyến nghị\s*\n)([\s\S]*)$/m,
    (_, heading: string, body: string) => {
      const lines = body
        .split('\n')
        .map((line) => line.trim())
        .filter(Boolean)
        .map((line) => (line.startsWith('- ') ? line : `- ${line}`));

      return `${heading}${lines.join('\n')}`;
    },
  );

  formatted = formatted.replace(
    /\b(\d+\s+(?:báo cáo độc lập|báo cáo|trường hợp tương tự|trường hợp|cảnh báo))\b/giu,
    '<span class="report-critical">$1</span>',
  );
  formatted = formatted.replace(
    /\b(rủi ro(?:\s+ở)?\s+mức rất cao|nguy cơ rất cao|Tuyệt đối không)\b/giu,
    '<span class="report-critical">$1</span>',
  );
  formatted = formatted.replace(
    /\b((?:VP Bank|Vietcombank|Techcombank|BIDV|Agribank|MB Bank|MBBank|ACB|TPBank|Sacombank))\b/gu,
    '<span class="report-emphasis">$1</span>',
  );
  formatted = formatted.replace(
    /\b(\d{8,})\b/gu,
    '<span class="report-emphasis">$1</span>',
  );
  formatted = formatted.replace(
    /(chủ tài khoản(?: ngân hàng)?(?:\s+[A-Z]{2,10})?(?:\s+số\s+<span class="report-emphasis">\d{8,}<\/span>)?(?:\s+tên)?\s+)([A-ZÀ-ỴĐ][\p{L}]+(?:\s+[A-ZÀ-ỴĐ][\p{L}]+){1,4})/gu,
    '$1<span class="report-emphasis">$2</span>',
  );

  formatted = formatted.replace(/__REPORT_SEGMENT_(\d+)__/g, (_, index: string) => {
    return preservedSegments[Number(index)] ?? '';
  });

  return formatted;
}
