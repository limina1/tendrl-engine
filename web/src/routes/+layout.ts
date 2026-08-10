// Token capture is a side-effect import and MUST stay first: it moves the
// host-injected auth_token from the URL into a cookie before any module
// fires an API request.
import '$lib/boot/token';

export const ssr = false;
export const prerender = false;
