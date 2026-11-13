// In-memory Google Drive (v3 + the v2 calls 5.18.0 and v6 use). Pure: `handle(request)` maps one HTTP request to one
// response, so it runs under Playwright routes (google.mjs) and in plain Node tests alike.
//
// Modelled on purpose (SYNC §23): the v2 etag changes on ANY write to a file (metadata or content), `version`
// increments on every write, `headRevisionId` only on content; `properties` merge key by key, null deletes, 124 bytes
// per key+value, 30 public properties; readers and commenters cannot write; `permissionIds` is returned to writers.
import crypto from 'node:crypto';

export const T0 = Date.parse('2026-10-30T10:00:00.000Z');
export const FOLDER = 'application/vnd.google-apps.folder';
const RANK = { reader: 1, commenter: 2, writer: 3, owner: 4 };
const iso = ms => new Date(ms).toISOString();
const bytes = value => Buffer.byteLength(String(value), 'utf8');

const fail = (status, reason, message, extra = {}) => ({
  status, json: { error: { code: status, message, errors: [{ domain: extra.domain || 'global', reason, message, ...(extra.location ? { locationType: 'header', location: extra.location } : {}) }] } }
});
const E = {
  auth: () => fail(401, 'authError', 'Invalid Credentials', { location: 'Authorization' }),
  notFound: id => fail(404, 'notFound', `File not found: ${id}.`),
  forbidden: () => fail(403, 'insufficientFilePermissions', 'The user does not have sufficient permissions for this file.'),
  precondition: () => fail(412, 'conditionNotMet', 'Precondition Failed', { location: 'If-Match' }),
  rate: (reason = 'userRateLimitExceeded') => fail(403, reason, reason === 'rateLimitExceeded' ? 'Rate Limit Exceeded' : 'User Rate Limit Exceeded', { domain: 'usageLimits' }),
  bad: message => fail(400, 'invalid', message),
  conflict: () => fail(409, 'fileIdInUse', 'A file already exists with the provided ID.')
};
export { E as driveErrors };

// ---- `fields` projection -------------------------------------------------------------------------------------------
function parseFields(text) {
  let i = 0;
  const item = () => {
    let name = '', sub = true;
    while (i < text.length && !',()/'.includes(text[i])) name += text[i++];
    if (text[i] === '(') { i++; sub = list(); i++; }
    else if (text[i] === '/') { i++; const [inner, innerSub] = item(); sub = { [inner]: innerSub }; }
    return [name.trim(), sub];
  };
  const list = () => {
    const out = {};
    for (;;) {
      const [name, sub] = item();
      if (name) out[name] = out[name] && out[name] !== true && sub !== true ? { ...out[name], ...sub } : sub;
      if (text[i] === ',') { i++; continue; }
      return out;
    }
  };
  return list();
}
function project(value, tree) {
  if (tree === true || tree['*']) return value;
  if (Array.isArray(value)) return value.map(item => project(item, tree));
  if (!value || typeof value !== 'object') return value;
  const out = {};
  for (const [key, sub] of Object.entries(tree)) if (value[key] !== undefined) out[key] = project(value[key], sub);
  return out;
}
export const applyFields = (value, fields, fallback) => project(value, parseFields(fields || fallback || '*'));

// ---- `q` search grammar --------------------------------------------------------------------------------------------
function tokenize(q) {
  const out = []; let i = 0;
  while (i < q.length) {
    const c = q[i];
    if (/\s/.test(c)) { i++; continue; }
    if (c === "'") { let s = ''; i++; while (i < q.length && q[i] !== "'") { if (q[i] === '\\') i++; s += q[i++]; } i++; out.push({ str: s }); continue; }
    if ('(){}'.includes(c)) { out.push({ op: c }); i++; continue; }
    const m = /^(!=|<=|>=|=|<|>)/.exec(q.slice(i));
    if (m) { out.push({ op: m[1] }); i += m[1].length; continue; }
    let word = ''; while (i < q.length && /[A-Za-z0-9_.:+-]/.test(q[i])) word += q[i++];
    if (!word) throw new Error(`Invalid query near "${q.slice(i, i + 12)}"`);
    out.push({ word });
  }
  return out;
}
/** Compiles a Drive `q` string into `(file, ctx) => boolean`; ctx gives `{me, roleOf, starred, sharedWithMe}`. */
export function compileQuery(q) {
  const t = tokenize(String(q || '')); let i = 0;
  const peek = () => t[i] || {}, next = () => t[i++] || {};
  const expect = op => { if (next().op !== op) throw new Error(`Invalid query: expected ${op}`); };
  const or = () => { let left = and(); while (peek().word === 'or') { i++; const a = left, b = and(); left = (f, c) => a(f, c) || b(f, c); } return left; };
  const and = () => { let left = unary(); while (peek().word === 'and') { i++; const a = left, b = unary(); left = (f, c) => a(f, c) && b(f, c); } return left; };
  const unary = () => {
    if (peek().word === 'not') { i++; const a = unary(); return (f, c) => !a(f, c); }
    if (peek().op === '(') { i++; const a = or(); expect(')'); return a; }
    return term();
  };
  const cmp = (op, a, b) => op === '=' ? a === b : op === '!=' ? a !== b : op === '<' ? a < b : op === '<=' ? a <= b : op === '>' ? a > b : a >= b;
  const term = () => {
    const first = next();
    if (first.str !== undefined) {
      if (next().word !== 'in') throw new Error('Invalid query: expected in');
      const field = next().word, value = first.str;
      if (field === 'parents') return (f, c) => (f.parents || []).includes(value === 'root' ? c.rootId : value);
      const want = field === 'owners' ? 4 : field === 'writers' ? 3 : 1;
      return (f, c) => { const email = value === 'me' ? c.me : value; const r = c.rankFor(f, email); return field === 'owners' ? r === 4 : field === 'writers' ? r >= 3 : r >= want; };
    }
    const field = first.word;
    if (field === 'sharedWithMe') {
      let want = true;
      if (peek().op === '=' || peek().op === '!=') { const o = next().op, v = next().word === 'true'; want = o === '=' ? v : !v; }
      return (f, c) => c.sharedWithMe(f) === want;
    }
    const op = next();
    if (op.word === 'has') {
      expect('{'); const pair = {};
      for (;;) { const k = next().word; expect('='); pair[k] = next().str; if (peek().word === 'and') { i++; continue; } break; }
      expect('}');
      return f => ((f[field] || {})[pair.key]) === pair.value;
    }
    const raw = next(), value = raw.str !== undefined ? raw.str : raw.word;
    if (op.word === 'contains') {
      const needle = String(value).toLowerCase();
      if (field === 'name') return f => { const n = f.name.toLowerCase(); let at = n.indexOf(needle); while (at >= 0) { if (at === 0 || !/[a-z0-9]/.test(n[at - 1])) return true; at = n.indexOf(needle, at + 1); } return false; };
      return f => f.name.toLowerCase().includes(needle);
    }
    if (!op.op) throw new Error('Invalid query: operator expected');
    if (field === 'trashed') return (f, c) => cmp(op.op, !!c.trashed(f), value === 'true');
    if (field === 'starred') return (f, c) => cmp(op.op, !!c.starred(f), value === 'true');
    if (/Time$/.test(field)) return f => cmp(op.op, f[field] || 0, Date.parse(value));
    return f => cmp(op.op, f[field], value);
  };
  if (!t.length) return () => true;
  const fn = or();
  if (i < t.length) throw new Error('Invalid query: trailing input');
  return fn;
}

// ---- multipart/related ---------------------------------------------------------------------------------------------
export function parseMultipart(body, contentType) {
  const boundary = /boundary=("?)([^";]+)\1/i.exec(contentType || '')?.[2];
  if (!boundary) return null;
  const mark = Buffer.from(`--${boundary}`), parts = [];
  let at = body.indexOf(mark);
  while (at >= 0) {
    const start = at + mark.length;
    if (body[start] === 45 && body[start + 1] === 45) break;
    const end = body.indexOf(mark, start);
    if (end < 0) break;
    let chunk = body.subarray(start, end);
    if (chunk[0] === 13) chunk = chunk.subarray(2); else if (chunk[0] === 10) chunk = chunk.subarray(1);
    let split = chunk.indexOf('\r\n\r\n'), gap = 4;
    if (split < 0) { split = chunk.indexOf('\n\n'); gap = 2; }
    const head = split < 0 ? '' : chunk.subarray(0, split).toString('latin1');
    let data = chunk.subarray(split < 0 ? 0 : split + gap);
    if (data.length >= 2 && data[data.length - 2] === 13 && data[data.length - 1] === 10) data = data.subarray(0, data.length - 2);
    else if (data.length && data[data.length - 1] === 10) data = data.subarray(0, data.length - 1);
    const type = /content-type:\s*([^\r\n]+)/i.exec(head)?.[1]?.trim() || '';
    if (/content-transfer-encoding:\s*base64/i.test(head)) data = Buffer.from(data.toString('latin1'), 'base64');
    parts.push({ type, data });
    at = end;
  }
  if (parts.length < 2) return null;
  return { meta: JSON.parse(parts[0].data.toString('utf8') || '{}'), data: Buffer.from(parts[1].data), type: parts[1].type };
}

// ---- the Drive -----------------------------------------------------------------------------------------------------
export class Drive {
  /**
   * @param {object} [options]
   * @param {() => number} [options.now] clock in ms; default is a logical clock: T0 + 1 s per write (deterministic)
   * @param {boolean} [options.metadataBumpsModifiedTime] properties/appProperties/starred-only writes also move modifiedTime
   */
  constructor(options = {}) {
    this.options = options;
    this.reset();
  }

  reset() {
    this.files = new Map();
    this.accounts = new Map();     // email → account
    this.tokens = new Map();       // access token → email
    this.refreshTokens = new Map();// refresh token → email
    this.sessions = new Map();     // resumable upload id → session
    this.serial = 0;
    this.clock = T0;
    this.log = [];                 // every handled request: {method, path, query, status, account}
    return this;
  }

  now() { return this.options.now ? this.options.now() : (this.clock += 1000); }
  /** Deterministic 33-character id shaped like a Drive id. */
  id(prefix = '1Mock') { return prefix + (++this.serial).toString(36).toUpperCase().padStart(33 - prefix.length, '0'); }

  addAccount({ email, displayName = email.split('@')[0], photoLink = '', token = `token-${email}`, refreshToken = `refresh-${email}` }) {
    const account = { email, displayName, photoLink, permissionId: String(10000000000000000000n + BigInt(this.accounts.size + 1)).slice(1), token, refreshToken, rootId: `root-${this.accounts.size + 1}`, starred: new Set(), usage: 0 };
    this.accounts.set(email, account);
    this.tokens.set(token, email);
    this.refreshTokens.set(refreshToken, email);
    return account;
  }
  /** Issues a fresh access token for the account (token endpoint, GIS stub). */
  issueToken(email) { const token = `token-${email}-${++this.serial}`; this.tokens.set(token, email); return token; }
  revokeToken(token) { this.tokens.delete(token); }

  /**
   * Seeds a file without going through HTTP. `owner` is an account email; `data` a Buffer for non-folders.
   * `permissions` entries: `{type:'user', role, emailAddress}` or `{type:'anyone', role}`.
   */
  addFile({ id = this.id(), name = 'Untitled', mimeType = 'application/pdf', owner, parents, data = null, properties = {}, appProperties = {},
    permissions = [], starred = false, trashed = false, folderColorRgb, modifiedTime, createdTime, resourceKey, revisionId } = {}) {
    const account = this.accounts.get(owner) || [...this.accounts.values()][0];
    if (!account) throw new Error('addFile needs an account');
    const at = modifiedTime != null ? Date.parse(modifiedTime) || modifiedTime : this.now();
    const file = { id, name, mimeType, owner: account.email, parents: (parents || ['root']).map(p => p === 'root' ? account.rootId : p),
      properties: { ...properties }, appProperties: { ...appProperties }, permissions: [], trashed: !!trashed, trashedTime: 0,
      folderColorRgb, resourceKey, version: 1, meta: ++this.serial, modifiedTime: at, createdTime: createdTime != null ? Date.parse(createdTime) || createdTime : at,
      lastModifier: account.email, revisions: [], comments: [] };
    file.permissions.push({ id: account.permissionId, type: 'user', role: 'owner', emailAddress: account.email, displayName: account.displayName, photoLink: account.photoLink });
    for (const p of permissions) this.#addPermission(file, p);
    if (mimeType !== FOLDER) this.#addRevision(file, Buffer.from(data || []), mimeType, account, at, revisionId);
    if (starred) account.starred.add(id);
    this.files.set(id, file);
    return file;
  }
  file(id) { return this.files.get(id); }
  /** Current content of a file (head revision). */
  content(id) { const f = this.files.get(id); return f?.revisions.at(-1)?.data || null; }

  #addRevision(file, data, mimeType, account, at, revisionId) {
    const revision = { id: revisionId || this.id('0BMockRev'), data, mimeType: mimeType || file.mimeType, modifiedTime: at, size: data.length,
      md5: crypto.createHash('md5').update(data).digest('hex'), by: account.email };
    file.revisions.push(revision);
    return revision;
  }
  #addPermission(file, p) {
    if (p.type === 'anyone') { const perm = { id: 'anyoneWithLink', type: 'anyone', role: p.role || 'reader', allowFileDiscovery: false }; file.permissions = file.permissions.filter(x => x.id !== perm.id).concat(perm); return perm; }
    const account = this.accounts.get(p.emailAddress);
    const perm = { id: account?.permissionId || String(20000000000000000000n + BigInt(++this.serial)).slice(1), type: p.type || 'user', role: p.role || 'reader',
      emailAddress: p.emailAddress, displayName: p.displayName || account?.displayName || String(p.emailAddress || '').split('@')[0], photoLink: p.photoLink || account?.photoLink || '' };
    file.permissions = file.permissions.filter(x => x.id !== perm.id).concat(perm);
    return perm;
  }
  #touch(file, { content = false, visible = true, account } = {}) {
    file.version += 1; file.meta = ++this.serial;
    if (content || visible || this.options.metadataBumpsModifiedTime) { file.modifiedTime = this.now(); if (account) file.lastModifier = account.email; }
  }

  // ---- access ----
  rank(file, email, seen = new Set()) {
    if (!file || seen.has(file.id)) return 0;
    seen.add(file.id);
    let best = 0;
    for (const p of file.permissions) {
      if (p.type === 'anyone' || (email && p.emailAddress === email)) best = Math.max(best, RANK[p.role] || 0);
    }
    for (const parent of file.parents || []) best = Math.max(best, Math.min(3, this.rank(this.files.get(parent), email, seen)) || 0);
    return best;
  }
  /** Rank without link permissions: what makes a file appear in a user's listings. */
  #listed(file, email, seen = new Set()) {
    if (!file || seen.has(file.id)) return false;
    seen.add(file.id);
    return file.permissions.some(p => p.type !== 'anyone' && p.emailAddress === email) || (file.parents || []).some(p => this.#listed(this.files.get(p), email, seen));
  }
  #trashed(file, seen = new Set()) {
    if (!file || seen.has(file.id)) return false;
    seen.add(file.id);
    return file.trashed || (file.parents || []).some(p => this.#trashed(this.files.get(p), seen));
  }
  etag(file) { return `"MockEtag${file.meta.toString(36).toUpperCase().padStart(8, '0')}"`; }

  #user(email, me) {
    const a = this.accounts.get(email);
    return { kind: 'drive#user', displayName: a?.displayName || email, emailAddress: email, photoLink: a?.photoLink || undefined, permissionId: a?.permissionId, me: !!me && me.email === email };
  }
  #resource(file, account) {
    const r = this.rank(file, account?.email), head = file.revisions.at(-1), folder = file.mimeType === FOLDER;
    const ext = /\.([^.]+)$/.exec(file.name)?.[1];
    return {
      kind: 'drive#file', id: file.id, name: file.name, mimeType: file.mimeType, parents: file.parents.slice(),
      trashed: this.#trashed(file), explicitlyTrashed: file.trashed, trashedTime: file.trashed ? iso(file.trashedTime) : undefined,
      starred: !!account?.starred.has(file.id), version: String(file.version), headRevisionId: head?.id,
      modifiedTime: iso(file.modifiedTime), createdTime: iso(file.createdTime), viewedByMeTime: iso(file.modifiedTime), modifiedByMeTime: iso(file.modifiedTime),
      size: head ? String(head.size) : undefined, quotaBytesUsed: head ? String(head.size) : '0', md5Checksum: head?.md5,
      originalFilename: folder ? undefined : file.name, fileExtension: folder ? undefined : ext || '', fullFileExtension: folder ? undefined : ext || '',
      properties: Object.keys(file.properties).length ? { ...file.properties } : undefined,
      appProperties: Object.keys(file.appProperties).length ? { ...file.appProperties } : undefined,
      ownedByMe: file.owner === account?.email, owners: [this.#user(file.owner, account)], lastModifyingUser: this.#user(file.lastModifier, account),
      shared: file.permissions.length > 1, spaces: ['drive'], folderColorRgb: folder ? file.folderColorRgb || '#8f8f8f' : undefined,
      resourceKey: file.resourceKey, writersCanShare: true, viewersCanCopyContent: true, copyRequiresWriterPermission: false,
      webViewLink: folder ? `https://drive.google.com/drive/folders/${file.id}` : `https://drive.google.com/file/d/${file.id}/view?usp=drivesdk`,
      webContentLink: folder ? undefined : `https://drive.google.com/uc?id=${file.id}&export=download`,
      iconLink: `https://drive-thirdparty.googleusercontent.com/16/type/${file.mimeType}`,
      hasThumbnail: !folder && !!head?.size, thumbnailLink: !folder && head?.size ? `https://lh3.googleusercontent.com/drive-storage/${file.id}-v${head.id}=s220` : undefined,
      sharedWithMeTime: file.owner !== account?.email && this.#listed(file, account?.email) ? iso(file.createdTime) : undefined,
      permissionIds: r >= 3 ? file.permissions.map(p => p.id) : undefined,
      permissions: r >= 3 ? file.permissions.map(p => this.#permission(p)) : undefined,
      capabilities: { canEdit: r >= 3, canModifyContent: r >= 3 && !folder, canComment: r >= 2, canShare: r >= 3, canCopy: !folder && r >= 1, canDownload: !folder && r >= 1,
        canRename: r >= 3, canTrash: r >= 4, canUntrash: r >= 4, canDelete: r >= 4, canAddChildren: folder && r >= 3, canListChildren: folder && r >= 1,
        canRemoveChildren: folder && r >= 3, canReadRevisions: !folder && r >= 3, canMoveItemWithinDrive: r >= 3, canMoveItemOutOfDrive: r >= 4, canAddMyDriveParent: false,
        canRemoveMyDriveParent: r >= 3, canChangeCopyRequiresWriterPermission: r >= 4, canChangeSecurityUpdateEnabled: false, canAcceptOwnership: false, canModifyLabels: r >= 3, canReadLabels: r >= 1 }
    };
  }
  #resourceV2(file, account) {
    const v3 = this.#resource(file, account);
    return { kind: 'drive#file', id: file.id, etag: this.etag(file), title: file.name, mimeType: file.mimeType, modifiedDate: v3.modifiedTime, createdDate: v3.createdTime,
      version: v3.version, headRevisionId: v3.headRevisionId, fileSize: v3.size, md5Checksum: v3.md5Checksum, editable: v3.capabilities.canEdit,
      labels: { starred: v3.starred, trashed: v3.trashed, hidden: false, restricted: false, viewed: true }, parents: file.parents.map(id => ({ kind: 'drive#parentReference', id, isRoot: /^root-/.test(id) })),
      properties: [...Object.entries(file.properties).map(([key, value]) => ({ kind: 'drive#property', key, value, visibility: 'PUBLIC' })),
        ...Object.entries(file.appProperties).map(([key, value]) => ({ kind: 'drive#property', key, value, visibility: 'PRIVATE' }))],
      capabilities: v3.capabilities, userPermission: { role: ['', 'reader', 'reader', 'writer', 'owner'][this.rank(file, account?.email)] } };
  }
  #permission(p) { return { kind: 'drive#permission', id: p.id, type: p.type, role: p.role, emailAddress: p.emailAddress, displayName: p.displayName, photoLink: p.photoLink || undefined, allowFileDiscovery: p.type === 'anyone' ? false : undefined, deleted: p.type === 'user' ? false : undefined }; }
  #revision(file, rev, account) { return { kind: 'drive#revision', id: rev.id, mimeType: rev.mimeType, modifiedTime: iso(rev.modifiedTime), size: String(rev.size), md5Checksum: rev.md5, keepForever: !!rev.keepForever, published: false, originalFilename: file.name, lastModifyingUser: this.#user(rev.by, account) }; }

  // ---- property rules ----
  #mergeProps(target, patch, { isPublic }) {
    if (patch == null) return null;
    if (typeof patch !== 'object' || Array.isArray(patch)) return E.bad('Invalid value for properties');
    const next = { ...target };
    for (const [key, value] of Object.entries(patch)) {
      if (value === null) { delete next[key]; continue; }
      if (bytes(key) + bytes(value) > 124) return fail(400, 'invalid', 'Property key and value must not exceed 124 bytes in UTF-8 encoding.');
      next[key] = String(value);
    }
    if (Object.keys(next).length > (isPublic ? 30 : 30)) return fail(403, 'propertyCountLimitExceeded', isPublic ? 'A file cannot have more than 30 public properties per application.' : 'A file cannot have more than 30 private properties per application.');
    return { next };
  }
  /** Applies a v3 metadata body. Returns an error response or null. */
  #applyMeta(file, body, account, query) {
    let visible = false;
    const stage = {};
    for (const [key, isPublic] of [['properties', true], ['appProperties', false]]) {
      if (!(key in body)) continue;
      const merged = this.#mergeProps(file[key], body[key], { isPublic });
      if (merged?.status) return merged;
      if (merged) stage[key] = merged.next;
    }
    Object.assign(file, stage);
    if (typeof body.name === 'string' && body.name !== file.name) { file.name = body.name; visible = true; }
    if (typeof body.mimeType === 'string' && !file.revisions.length) file.mimeType = body.mimeType;
    if (typeof body.trashed === 'boolean' && body.trashed !== file.trashed) { file.trashed = body.trashed; file.trashedTime = body.trashed ? this.now() : 0; visible = true; }
    if (typeof body.starred === 'boolean') { if (body.starred) account.starred.add(file.id); else account.starred.delete(file.id); }
    if ('folderColorRgb' in body) file.folderColorRgb = body.folderColorRgb || undefined;
    const root = id => id === 'root' ? account.rootId : id;
    const remove = (query.get('removeParents') || '').split(',').filter(Boolean).map(root), add = (query.get('addParents') || '').split(',').filter(Boolean).map(root);
    if (remove.length || add.length) { file.parents = file.parents.filter(p => !remove.includes(p)).concat(add.filter(p => !file.parents.includes(p))); visible = true; }
    this.#touch(file, { visible, account });
    if (typeof body.modifiedTime === 'string' && Number.isFinite(Date.parse(body.modifiedTime))) file.modifiedTime = Date.parse(body.modifiedTime);
    return null;
  }
  #create(meta, account, data, type) {
    if (meta.id && this.files.has(meta.id)) return E.conflict();
    const mimeType = meta.mimeType || type || 'application/octet-stream';
    const parents = (Array.isArray(meta.parents) && meta.parents.length ? meta.parents : ['root']).map(p => p === 'root' ? account.rootId : p);
    for (const parent of parents) {
      if (/^root-/.test(parent)) continue;
      const folder = this.files.get(parent);
      if (!folder) return E.notFound(parent);
      if (this.rank(folder, account.email) < 3) return E.forbidden();
    }
    const props = {};
    for (const [key, isPublic] of [['properties', true], ['appProperties', false]]) {
      const merged = this.#mergeProps({}, meta[key], { isPublic });
      if (merged?.status) return merged;
      props[key] = merged?.next || {};
    }
    const file = this.addFile({ id: meta.id || this.id(), name: meta.name ?? 'Untitled', mimeType, owner: account.email, parents, data: data || Buffer.alloc(0), ...props, starred: !!meta.starred, folderColorRgb: meta.folderColorRgb });
    if (typeof meta.modifiedTime === 'string' && Number.isFinite(Date.parse(meta.modifiedTime))) file.modifiedTime = Date.parse(meta.modifiedTime);
    if (typeof meta.createdTime === 'string' && Number.isFinite(Date.parse(meta.createdTime))) file.createdTime = Date.parse(meta.createdTime);
    return { file };
  }
  #setContent(file, data, type, account) {
    this.#addRevision(file, data, type || file.mimeType, account, 0);
    this.#touch(file, { content: true, account });
    file.revisions.at(-1).modifiedTime = file.modifiedTime;
    if (file.revisions.length > 100) file.revisions.splice(0, file.revisions.length - 100);
  }

  // ---- HTTP ----
  /** @param {{method:string, url:URL|string, headers?:Record<string,string>, body?:Buffer}} request */
  handle(request) {
    const url = typeof request.url === 'string' ? new URL(request.url) : request.url;
    const headers = Object.fromEntries(Object.entries(request.headers || {}).map(([k, v]) => [k.toLowerCase(), v]));
    const body = request.body || Buffer.alloc(0), method = request.method.toUpperCase();
    const bearer = /^Bearer\s+(.+)$/i.exec(headers.authorization || '')?.[1];
    const account = this.accounts.get(this.tokens.get(bearer)) || null;
    let out;
    try { out = this.#route(method, url, headers, body, account, !!bearer); }
    catch (error) { out = error?.status ? error : E.bad(String(error?.message || error)); }
    const response = { status: out.status || 200, headers: { ...(out.headers || {}) }, body: out.body ?? null };
    if (out.json !== undefined) { response.body = Buffer.from(JSON.stringify(out.json)); response.headers['Content-Type'] = 'application/json; charset=UTF-8'; }
    else if (response.body == null) response.body = Buffer.alloc(0);
    this.log.push({ method, path: url.pathname, query: url.search, status: response.status, account: account?.email || null, bytes: body.length + response.body.length });
    return response;
  }

  #json(text) { if (!text.length) return {}; try { const v = JSON.parse(text.toString('utf8')); return v && typeof v === 'object' ? v : {}; } catch { throw E.bad('Parse Error'); } }

  #route(method, url, headers, body, account, hadToken) {
    const path = url.pathname, q = url.searchParams, fields = q.get('fields');
    const upload = path.startsWith('/upload/drive/'), v2 = /^\/(upload\/)?drive\/v2\//.test(path);
    const rest = path.replace(/^\/(upload\/)?drive\/v[23]\//, '').split('/').filter(Boolean).map(decodeURIComponent);
    if (!/^\/(upload\/)?drive\/v[23]\//.test(path)) return fail(404, 'notFound', 'Not Found');
    if (q.has('upload_id')) return this.#resumableChunk(method, q, headers, body);
    if (!account) {
      // Anonymous API access exists only for link-shared files with an API key (5.18.0 ships none; v6 never sends one).
      if (!hadToken && q.has('key') && rest[0] === 'files' && rest[1] && method === 'GET') {
        const file = this.files.get(rest[1]);
        if (!file || this.rank(file, null) < 1) return E.notFound(rest[1]);
        if (q.get('alt') === 'media') return this.#media(file, file.revisions.at(-1), headers);
        return { json: applyFields(this.#resource(file, null), fields, 'kind,id,name,mimeType') };
      }
      return hadToken ? E.auth() : fail(401, 'required', 'Login Required', { location: 'Authorization' });
    }
    const out = (file, status = 200) => ({ status, json: v2 ? applyFields(this.#resourceV2(file, account), fields) : applyFields(this.#resource(file, account), fields, 'kind,id,name,mimeType'), headers: { ETag: this.etag(file) } });

    if (rest[0] === 'about') {
      const used = [...this.files.values()].filter(f => f.owner === account.email).reduce((n, f) => n + (f.revisions.at(-1)?.size || 0), 0);
      if (!fields) return fail(400, 'required', "The 'fields' parameter is required for this method.");
      return { json: applyFields({ kind: 'drive#about', user: this.#user(account.email, account), storageQuota: { limit: '16106127360', usage: String(used), usageInDrive: String(used), usageInDriveTrash: '0' },
        maxUploadSize: '5242880000000', canCreateDrives: false, appInstalled: true }, fields) };
    }
    if (rest[0] !== 'files') return fail(404, 'notFound', 'Not Found');
    const idArg = rest[1] === 'root' ? account.rootId : rest[1];

    // files.list
    if (rest.length === 1 && method === 'GET') {
      let match;
      try { match = compileQuery(q.get('q')); } catch (error) { return E.bad(error.message); }
      const ctx = { me: account.email, rootId: account.rootId, rankFor: (f, email) => this.rank(f, email), trashed: f => this.#trashed(f), starred: f => account.starred.has(f.id),
        sharedWithMe: f => f.owner !== account.email && f.permissions.some(p => p.emailAddress === account.email) };
      let rows = [...this.files.values()].filter(f => (f.owner === account.email || this.#listed(f, account.email)) && match(f, ctx));
      const order = (q.get('orderBy') || '').split(',').map(s => s.trim()).filter(Boolean).map(s => { const [key, dir] = s.split(/\s+/); return { key, desc: dir === 'desc' }; });
      const keyOf = (f, key) => key === 'folder' ? (f.mimeType === FOLDER ? 0 : 1) : key === 'name' || key === 'name_natural' ? f.name.toLowerCase()
        : key === 'starred' ? (account.starred.has(f.id) ? 0 : 1) : key === 'quotaBytesUsed' ? (f.revisions.at(-1)?.size || 0)
        : key === 'createdTime' ? f.createdTime : f.modifiedTime;
      if (order.length) rows.sort((a, b) => { for (const { key, desc } of order) { const x = keyOf(a, key), y = keyOf(b, key); if (x !== y) { const c = key === 'name_natural' ? String(x).localeCompare(String(y), 'en', { numeric: true }) : x < y ? -1 : 1; return desc ? -c : c; } } return a.id < b.id ? -1 : a.id > b.id ? 1 : 0; });
      const size = Math.max(1, Math.min(1000, Number(q.get('pageSize')) || 100)), from = Number((q.get('pageToken') || '').replace(/^page-/, '')) || 0;
      const page = rows.slice(from, from + size);
      return { json: applyFields({ kind: 'drive#fileList', incompleteSearch: false, nextPageToken: from + size < rows.length ? `page-${from + size}` : undefined, files: page.map(f => this.#resource(f, account)) }, fields, 'kind,incompleteSearch,nextPageToken,files(kind,id,name,mimeType)') };
    }
    // files.generateIds
    if (rest[1] === 'generateIds') {
      const count = Math.max(1, Math.min(1000, Number(q.get('count')) || 10));
      return { json: { kind: 'drive#generatedIds', space: q.get('space') || 'drive', ids: Array.from({ length: count }, () => this.id('1MockGen')) } };
    }
    // files.create (metadata only, multipart, media, resumable)
    if (rest.length === 1 && method === 'POST') {
      const type = q.get('uploadType');
      if (upload && type === 'resumable') return this.#resumableStart({ create: true, meta: this.#json(body), headers, url, account, v2 });
      let meta = {}, data = null, mime = '';
      if (upload && type === 'multipart') { const parts = parseMultipart(body, headers['content-type']); if (!parts) return E.bad('Bad multipart body'); ({ meta, data } = parts); mime = parts.type; }
      else if (upload) { data = body; mime = headers['content-type'] || ''; }
      else meta = this.#json(body);
      const made = this.#create(meta, account, data, mime);
      return made.status ? made : out(made.file);
    }
    if (rest[1] === 'trash' && method === 'DELETE') {
      for (const f of [...this.files.values()]) if (f.owner === account.email && f.trashed) this.#delete(f);
      return { status: 204 };
    }

    const file = this.files.get(idArg);
    if (idArg && /^root-/.test(idArg) && rest.length === 2 && method === 'GET') {
      return { json: applyFields({ kind: 'drive#file', id: idArg, name: 'My Drive', mimeType: FOLDER, parents: [], ownedByMe: true, capabilities: { canEdit: true, canAddChildren: true } }, fields, 'kind,id,name,mimeType') };
    }
    const rank = this.rank(file, account.email);
    if (!file || rank < 1) return E.notFound(idArg);

    // files.copy
    if (rest[2] === 'copy' && method === 'POST') {
      const meta = this.#json(body);
      if (file.mimeType === FOLDER) return fail(403, 'cannotCopyFile', 'This file cannot be copied by the user.');
      // The copy inherits the source's properties and appProperties; keys in the request override, null removes.
      const made = this.#create({ name: meta.name ?? `Copy of ${file.name}`, mimeType: file.mimeType, parents: meta.parents || file.parents.filter(p => this.rank(this.files.get(p), account.email) >= 3 || p === account.rootId),
        properties: { ...file.properties, ...(meta.properties || {}) }, appProperties: { ...file.appProperties, ...(meta.appProperties || {}) }, id: meta.id, starred: meta.starred }, account, Buffer.from(file.revisions.at(-1).data), file.mimeType);
      return made.status ? made : out(made.file);
    }
    // permissions
    if (rest[2] === 'permissions') {
      const permId = rest[3];
      if (!permId && method === 'GET') { if (rank < 3) return E.forbidden(); return { json: applyFields({ kind: 'drive#permissionList', permissions: file.permissions.map(p => this.#permission(p)) }, fields, 'kind,permissions(kind,id,type,role)') }; }
      if (rank < 3) return E.forbidden();
      if (!permId && method === 'POST') {
        const p = this.#json(body);
        if (!RANK[p.role] || p.role === 'owner') return E.bad('The specified role is invalid.');
        if (p.type !== 'anyone' && !p.emailAddress) return E.bad('The permission type field is required.');
        const perm = this.#addPermission(file, p);
        this.#touch(file, { visible: false, account });
        return { json: applyFields(this.#permission(perm), fields, 'kind,id,type,role') };
      }
      const perm = file.permissions.find(p => p.id === permId);
      if (!perm) return fail(404, 'notFound', `Permission not found: ${permId}.`);
      if (method === 'GET') return { json: applyFields(this.#permission(perm), fields, 'kind,id,type,role') };
      if (perm.role === 'owner') return fail(403, 'cannotModifyOwnerPermission', 'The owner permission cannot be changed.');
      if (method === 'PATCH') {
        const p = this.#json(body);
        if (!RANK[p.role] || p.role === 'owner') return E.bad('The specified role is invalid.');
        perm.role = p.role; this.#touch(file, { visible: false, account });
        return { json: applyFields(this.#permission(perm), fields, 'kind,id,type,role') };
      }
      if (method === 'DELETE') { file.permissions = file.permissions.filter(p => p !== perm); this.#touch(file, { visible: false, account }); return { status: 204 }; }
    }
    // revisions
    if (rest[2] === 'revisions') {
      if (rank < 3 && q.get('alt') !== 'media') return E.forbidden();
      if (!rest[3]) return { json: applyFields({ kind: 'drive#revisionList', revisions: file.revisions.map(r => this.#revision(file, r, account)) }, fields, 'kind,nextPageToken,revisions(kind,id,mimeType,modifiedTime)') };
      const rev = rest[3] === 'head' ? file.revisions.at(-1) : file.revisions.find(r => r.id === rest[3]);
      if (!rev) return fail(404, 'revisionNotFound', `Revision not found: ${rest[3]}.`);
      if (method === 'GET') return q.get('alt') === 'media' ? this.#media(file, rev, headers) : { json: applyFields(this.#revision(file, rev, account), fields, 'kind,id,mimeType,modifiedTime') };
      if (method === 'PATCH') { const p = this.#json(body); if ('keepForever' in p) rev.keepForever = !!p.keepForever; return { json: applyFields(this.#revision(file, rev, account), fields, 'kind,id,mimeType,modifiedTime') }; }
      if (method === 'DELETE') { if (file.revisions.length < 2) return fail(400, 'cannotDeleteOnlyRevision', 'Cannot delete the only revision.'); file.revisions = file.revisions.filter(r => r !== rev); return { status: 204 }; }
    }
    // comments and replies
    if (rest[2] === 'comments') return this.#comments(file, rest, method, q, body, account, rank);

    if (rest.length !== 2) return fail(404, 'notFound', 'Not Found');
    if (method === 'GET') return q.get('alt') === 'media' ? this.#media(file, file.revisions.at(-1), headers) : out(file);
    if (method === 'DELETE') { if (rank < 4) return E.forbidden(); this.#delete(file); return { status: 204 }; }
    if (method === 'PATCH' || method === 'PUT') {
      if (rank < 3) return E.forbidden();
      if (headers['if-match'] && headers['if-match'] !== '*' && headers['if-match'] !== this.etag(file)) return E.precondition();
      const type = q.get('uploadType');
      if (upload && type === 'resumable') return this.#resumableStart({ file, meta: this.#json(body), headers, url, account, v2 });
      let meta = {}, data = null, mime = '';
      if (upload && type === 'multipart') { const parts = parseMultipart(body, headers['content-type']); if (!parts) return E.bad('Bad multipart body'); ({ meta, data } = parts); mime = parts.type; }
      else if (upload) { data = body; mime = headers['content-type'] || ''; }
      else meta = this.#json(body);
      if (v2 && typeof meta.title === 'string') meta.name = meta.title;
      if ((meta.trashed === true || meta.trashed === false) && rank < 4) return E.forbidden();
      if (Object.keys(meta).length || q.has('addParents') || q.has('removeParents') || !data) { const bad = this.#applyMeta(file, meta, account, q); if (bad) return bad; }
      if (data) { if (file.mimeType === FOLDER) return E.bad('Folders have no content'); this.#setContent(file, Buffer.from(data), mime, account); }
      return out(file);
    }
    return fail(405, 'httpMethodNotAllowed', 'Method Not Allowed');
  }

  #delete(file) {
    this.files.delete(file.id);
    for (const child of [...this.files.values()]) if (child.parents.includes(file.id)) this.#delete(child);
  }

  #media(file, rev, headers) {
    if (!rev) return fail(403, 'fileNotDownloadable', 'Only files with binary content can be downloaded.');
    const base = { 'Content-Type': rev.mimeType || file.mimeType, 'Accept-Ranges': 'bytes', ETag: this.etag(file), 'X-Goog-Hash': `md5=${Buffer.from(rev.md5, 'hex').toString('base64')}` };
    const range = /^bytes=(\d*)-(\d*)$/.exec(headers.range || '');
    if (range && (range[1] || range[2])) {
      let start = range[1] ? Number(range[1]) : Math.max(0, rev.size - Number(range[2])), end = range[1] && range[2] ? Number(range[2]) : rev.size - 1;
      end = Math.min(end, rev.size - 1);
      if (start > end) return { status: 416, headers: { 'Content-Range': `bytes */${rev.size}` } };
      return { status: 206, headers: { ...base, 'Content-Range': `bytes ${start}-${end}/${rev.size}` }, body: rev.data.subarray(start, end + 1) };
    }
    return { status: 200, headers: base, body: rev.data };
  }

  // Resumable upload: initiation returns a session URI in `Location`; chunks are PUT with `Content-Range`.
  #resumableStart({ create, file, meta, headers, url, account, v2 }) {
    if (v2 && typeof meta.title === 'string') meta.name = meta.title;
    if (create && meta.id && this.files.has(meta.id)) return E.conflict();
    const id = `MockUpload${(++this.serial).toString(36).toUpperCase().padStart(8, '0')}`;
    this.sessions.set(id, { id, create: !!create, fileId: file?.id, meta, account: account.email, v2, fields: url.searchParams.get('fields'), query: url.searchParams,
      type: headers['x-upload-content-type'] || '', total: headers['x-upload-content-length'] != null ? Number(headers['x-upload-content-length']) : null,
      ifMatch: headers['if-match'] || null, chunks: [], received: 0 });
    const location = new URL(url); location.searchParams.set('upload_id', id);
    return { status: 200, headers: { Location: location.toString(), 'X-GUploader-UploadID': id } };
  }
  #resumableChunk(method, q, headers, body) {
    const session = this.sessions.get(q.get('upload_id'));
    if (!session) return fail(404, 'notFound', 'Upload session not found or expired.');
    if (method === 'DELETE') { this.sessions.delete(session.id); return { status: 499 }; }
    const account = this.accounts.get(session.account);
    const range = /^bytes\s+(\*|(\d+)-(\d+))\/(\*|\d+)$/.exec(headers['content-range'] || '');
    const incomplete = () => ({ status: 308, headers: session.received ? { Range: `bytes=0-${session.received - 1}`, 'X-Range-MD5': '' } : {} });
    if (range && range[4] !== '*') session.total = Number(range[4]);
    if (range && range[1] === '*') { if (session.total != null && session.received >= session.total) return this.#resumableFinish(session, account); return incomplete(); }
    const start = range ? Number(range[2]) : 0;
    if (start !== session.received) return incomplete();       // wrong offset: the client must resume from `Range`
    session.chunks.push(Buffer.from(body)); session.received += body.length;
    if (range && session.total != null && Number(range[3]) + 1 !== session.received) return E.bad('Invalid Content-Range');
    if (!range) session.total = session.received;
    if (session.total == null || session.received < session.total) return incomplete();
    return this.#resumableFinish(session, account);
  }
  #resumableFinish(session, account) {
    this.sessions.delete(session.id);
    const data = Buffer.concat(session.chunks);
    let file;
    if (session.create) { const made = this.#create(session.meta, account, data, session.type); if (made.status) return made; file = made.file; }
    else {
      file = this.files.get(session.fileId);
      if (!file) return E.notFound(session.fileId);
      if (this.rank(file, account.email) < 3) return E.forbidden();
      if (session.ifMatch && session.ifMatch !== '*' && session.ifMatch !== this.etag(file)) return E.precondition();
      if (Object.keys(session.meta).length) { const bad = this.#applyMeta(file, session.meta, account, session.query); if (bad) return bad; }
      this.#setContent(file, data, session.type, account);
    }
    return { status: session.create ? 200 : 200, json: session.v2 ? applyFields(this.#resourceV2(file, account), session.fields) : applyFields(this.#resource(file, account), session.fields, 'kind,id,name,mimeType'), headers: { ETag: this.etag(file) } };
  }

  #comments(file, rest, method, q, body, account, rank) {
    const fields = q.get('fields'), commentId = rest[3], replies = rest[4] === 'replies', replyId = rest[5];
    if (!fields && method !== 'DELETE') return fail(400, 'required', 'The \'fields\' parameter is required for this method.');
    const view = c => ({ kind: 'drive#comment', id: c.id, createdTime: iso(c.createdTime), modifiedTime: iso(c.modifiedTime), author: this.#user(c.by, account), content: c.content, htmlContent: c.content,
      deleted: !!c.deleted, resolved: !!c.resolved, anchor: c.anchor, quotedFileContent: c.quotedFileContent, replies: c.replies.filter(r => !r.deleted).map(replyView) });
    const replyView = r => ({ kind: 'drive#reply', id: r.id, createdTime: iso(r.createdTime), modifiedTime: iso(r.modifiedTime), author: this.#user(r.by, account), content: r.content, htmlContent: r.content, deleted: !!r.deleted, action: r.action });
    if (!commentId) {
      if (method === 'GET') {
        const since = q.get('startModifiedTime') ? Date.parse(q.get('startModifiedTime')) : 0, withDeleted = q.get('includeDeleted') === 'true';
        const rows = file.comments.filter(c => (withDeleted || !c.deleted) && c.modifiedTime >= since).sort((a, b) => b.modifiedTime - a.modifiedTime || (a.id < b.id ? 1 : -1));
        const size = Math.max(1, Math.min(100, Number(q.get('pageSize')) || 20)), from = Number((q.get('pageToken') || '').replace(/^page-/, '')) || 0;
        return { json: applyFields({ kind: 'drive#commentList', comments: rows.slice(from, from + size).map(view), nextPageToken: from + size < rows.length ? `page-${from + size}` : undefined }, fields) };
      }
      if (method === 'POST') {
        if (rank < 2) return E.forbidden();
        const p = this.#json(body), at = this.now();
        if (typeof p.content !== 'string') return E.bad('Comment content is required.');
        const c = { id: this.id('AAAAMockCmt').slice(0, 22), createdTime: at, modifiedTime: at, by: account.email, content: p.content, anchor: p.anchor, quotedFileContent: p.quotedFileContent, replies: [] };
        file.comments.push(c);
        return { json: applyFields(view(c), fields) };
      }
    }
    const c = file.comments.find(x => x.id === commentId && !x.deleted);
    if (!c) return fail(404, 'notFound', `Comment not found: ${commentId}`);
    if (replies) {
      if (!replyId && method === 'GET') return { json: applyFields({ kind: 'drive#replyList', replies: c.replies.filter(r => !r.deleted).map(replyView) }, fields) };
      if (!replyId && method === 'POST') {
        if (rank < 2) return E.forbidden();
        const p = this.#json(body), at = this.now();
        const r = { id: this.id('AAAAMockRep').slice(0, 22), createdTime: at, modifiedTime: at, by: account.email, content: String(p.content ?? ''), action: p.action };
        c.replies.push(r); c.modifiedTime = at; if (p.action === 'resolve') c.resolved = true; if (p.action === 'reopen') c.resolved = false;
        return { json: applyFields(replyView(r), fields) };
      }
      const r = c.replies.find(x => x.id === replyId && !x.deleted);
      if (!r) return fail(404, 'notFound', `Reply not found: ${replyId}`);
      if (method === 'GET') return { json: applyFields(replyView(r), fields) };
      if (r.by !== account.email) return E.forbidden();
      if (method === 'PATCH') { r.content = String(this.#json(body).content ?? r.content); r.modifiedTime = c.modifiedTime = this.now(); return { json: applyFields(replyView(r), fields) }; }
      if (method === 'DELETE') { r.deleted = true; c.modifiedTime = this.now(); return { status: 204 }; }
    }
    if (method === 'GET') return { json: applyFields(view(c), fields) };
    if (c.by !== account.email && rank < 4) return E.forbidden();
    if (method === 'PATCH') { const p = this.#json(body); if (typeof p.content === 'string') c.content = p.content; if ('resolved' in p) c.resolved = !!p.resolved; c.modifiedTime = this.now(); return { json: applyFields(view(c), fields) }; }
    if (method === 'DELETE') { c.deleted = true; c.modifiedTime = this.now(); return { status: 204 }; }
    return fail(405, 'httpMethodNotAllowed', 'Method Not Allowed');
  }
}
