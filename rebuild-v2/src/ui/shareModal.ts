import type { SyncMachine } from '../sync/syncMachine';
import { createPublicLink, getShareLink, shareDoc } from '../sync/shares';
import { makeOverlay, buttonStyle, smallText } from './modal';

export class ShareModal {
  constructor(
    private host: HTMLElement,
    private sync: SyncMachine | null
  ) {}

  open(): void {
    const modal = makeOverlay('Share');
    this.host.appendChild(modal.overlay);
    this.render(modal.body);
  }

  private render(body: HTMLElement): void {
    body.innerHTML = '';
    if (!this.sync) {
      body.appendChild(smallText('Drive sync is not available for this document.'));
      return;
    }

    const linkBox = document.createElement('div');
    Object.assign(linkBox.style, sectionStyle());
    const linkTitle = document.createElement('div');
    linkTitle.textContent = 'Link';
    Object.assign(linkTitle.style, titleStyle());
    linkBox.appendChild(linkTitle);
    const linkOut = document.createElement('input');
    linkOut.readOnly = true;
    linkOut.placeholder = 'Create a link to share this document';
    Object.assign(linkOut.style, inputStyle());
    linkBox.appendChild(linkOut);
    const linkActions = document.createElement('div');
    Object.assign(linkActions.style, { display: 'flex', gap: '8px', marginTop: '8px' });
    const create = document.createElement('button');
    create.type = 'button';
    create.textContent = 'Create Link';
    Object.assign(create.style, buttonStyle('primary'));
    create.addEventListener('click', async () => {
      create.disabled = true;
      create.textContent = 'Working';
      try {
        const file = await this.sync!.currentDriveFile();
        if (!file) throw new Error('Drive file was not created');
        await Promise.all(this.sync!.driveAssetFileIds().map((fileId) =>
          createPublicLink(this.sync!.drive, fileId)
        ));
        linkOut.value = await createPublicLink(this.sync!.drive, file.id);
      } catch (err) {
        linkOut.value = err instanceof Error ? err.message : String(err);
      } finally {
        create.disabled = false;
        create.textContent = 'Create Link';
      }
    });
    const copy = document.createElement('button');
    copy.type = 'button';
    copy.textContent = 'Copy';
    Object.assign(copy.style, buttonStyle());
    copy.addEventListener('click', async () => {
      if (!linkOut.value) {
        const file = await this.sync!.currentDriveFile();
        if (!file) return;
        const share = await getShareLink(this.sync!.drive, file.id);
        linkOut.value = share.link;
      }
      await navigator.clipboard?.writeText(linkOut.value);
    });
    linkActions.append(create, copy);
    linkBox.appendChild(linkActions);
    body.appendChild(linkBox);

    const emailBox = document.createElement('form');
    Object.assign(emailBox.style, sectionStyle());
    const emailTitle = document.createElement('div');
    emailTitle.textContent = 'Invite';
    Object.assign(emailTitle.style, titleStyle());
    emailBox.appendChild(emailTitle);
    const email = document.createElement('input');
    email.type = 'email';
    email.placeholder = 'name@example.com';
    Object.assign(email.style, inputStyle());
    const role = document.createElement('select');
    for (const value of ['writer', 'reader', 'commenter'] as const) {
      const option = document.createElement('option');
      option.value = value;
      option.textContent = value;
      role.appendChild(option);
    }
    Object.assign(role.style, inputStyle());
    const send = document.createElement('button');
    send.type = 'submit';
    send.textContent = 'Send Invite';
    Object.assign(send.style, buttonStyle('primary'));
    const result = smallText('');
    emailBox.append(email, role, send, result);
    emailBox.addEventListener('submit', async (event) => {
      event.preventDefault();
      if (!email.value.trim()) return;
      send.disabled = true;
      send.textContent = 'Sending';
      try {
        const file = await this.sync!.currentDriveFile();
        if (!file) throw new Error('Drive file was not created');
        await Promise.all(this.sync!.driveAssetFileIds().map((fileId) =>
          shareDoc(this.sync!.drive, fileId, email.value.trim(), role.value as 'writer' | 'reader' | 'commenter')
        ));
        await shareDoc(this.sync!.drive, file.id, email.value.trim(), role.value as 'writer' | 'reader' | 'commenter');
        result.textContent = 'Invite sent.';
      } catch (err) {
        result.textContent = err instanceof Error ? err.message : String(err);
      } finally {
        send.disabled = false;
        send.textContent = 'Send Invite';
      }
    });
    body.appendChild(emailBox);
  }
}

function sectionStyle(): Partial<CSSStyleDeclaration> {
  return {
    display: 'grid',
    gap: '8px',
    padding: '12px',
    border: '1px solid #e2e8f0',
    borderRadius: '8px',
    background: '#fff',
    marginBottom: '12px'
  };
}

function titleStyle(): Partial<CSSStyleDeclaration> {
  return { fontSize: '14px', fontWeight: '800', color: '#0f172a' };
}

function inputStyle(): Partial<CSSStyleDeclaration> {
  return {
    height: '34px',
    border: '1px solid #cbd5e1',
    borderRadius: '7px',
    padding: '0 10px',
    minWidth: '0'
  };
}
