import type { PageLoad } from './$types';

const VALID_TYPES = new Set(['phone', 'bank', 'url']);

export const load: PageLoad = ({ url }) => {
    const type = url.searchParams.get('type') ?? 'phone';
    return {
        q: url.searchParams.get('q') ?? '',
        type: VALID_TYPES.has(type) ? type : 'phone',
    };
};
