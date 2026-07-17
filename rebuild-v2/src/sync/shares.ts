// Google Drive sharing permissions.
// Handles sharing documents, creating public links, and revoking access.

import type { DriveClient, DriveFile } from './driveClient';

// ── Permission roles ───────────────────────────────────────────────────────────

export type PermissionRole = 'reader' | 'commenter' | 'writer' | 'owner';

/** Who the permission applies to. */
export interface PermissionTarget {
  type: 'user' | 'group' | 'domain' | 'anyone';
  email?: string;
  domain?: string;
}

export interface Permission {
  id: string;
  type: string;
  role: PermissionRole;
  emailAddress?: string;
  displayName?: string;
}

// ── Share helpers ───────────────────────────────────────────────────────────────

/**
 * Share a document with a specific email address.
 * @param drive DriveClient instance
 * @param fileId ID of the file to share
 * @param email Email address of the recipient
 * @param role Permission role to grant ('reader', 'commenter', or 'writer')
 */
export async function shareDoc(
  drive: DriveClient,
  fileId: string,
  email: string,
  role: PermissionRole = 'writer'
): Promise<Permission> {
  if (role === 'owner') {
    throw new Error('Cannot transfer ownership via API — use the Drive UI');
  }

  return drive.post<Permission>(`/files/${fileId}/permissions`, {
    type: 'user',
    role,
    emailAddress: email
  }, {
    sendNotificationEmail: 'true',
    fields: 'id,type,role,emailAddress,displayName'
  });
}

/**
 * Create a public link (anyone with the link can view).
 * @param drive DriveClient instance
 * @param fileId ID of the file to make public
 */
export async function createPublicLink(
  drive: DriveClient,
  fileId: string
): Promise<string> {
  // Create an "anyone" permission with reader role
  const permission = await drive.post<Permission>(`/files/${fileId}/permissions`, {
    type: 'anyone',
    role: 'reader'
  }, {
    fields: 'id'
  });

  // Set the file to be publicly accessible
  await drive.patch<DriveFile>(`/files/${fileId}`, {
    viewersCanCopyContent: false
  });

  return `https://drive.google.com/file/d/${fileId}/view`;
}

/**
 * Revoke all sharing for a file.
 * @param drive DriveClient instance
 * @param fileId ID of the file
 */
export async function revokeSharing(drive: DriveClient, fileId: string): Promise<void> {
  // Get all permissions
  const result = await drive.get<{ permissions: Permission[] }>(`/files/${fileId}/permissions`, {
    fields: 'permissions(id,type,role)'
  });

  // Delete all non-owner permissions
  const ownerRole = 'owner';
  for (const permission of result.permissions) {
    // Don't delete owner permission or the current user's permission
    if (permission.role !== ownerRole) {
      await drive.delete(`/files/${fileId}/permissions/${permission.id}`);
    }
  }
}

/**
 * Get the resource key for a shared file.
 * Resource keys are required to access files shared via private links.
 * @param drive DriveClient instance
 * @param fileId ID of the file
 */
export async function getResourceKey(drive: DriveClient, fileId: string): Promise<string | null> {
  try {
    const file = await drive.get<DriveFile>(`/files/${fileId}`, {
      fields: 'resourceKey'
    });
    return file.resourceKey ?? null;
  } catch {
    return null;
  }
}

/**
 * Make a file copy in the user's Drive.
 * Useful for "Save a copy" functionality.
 * @param drive DriveClient instance
 * @param fileId ID of the source file
 * @param copyName Name for the new copy
 */
export async function copyFile(
  drive: DriveClient,
  fileId: string,
  copyName: string
): Promise<DriveFile> {
  return drive.post<DriveFile>(`/files/${fileId}/copy`, {
    name: copyName,
    parents: ['root']
  }, {
    fields: 'id,name,mimeType,modifiedTime'
  });
}

/**
 * Get the sharing link for a file, creating one if necessary.
 * @param drive DriveClient instance
 * @param fileId ID of the file
 */
export async function getShareLink(
  drive: DriveClient,
  fileId: string
): Promise<{ link: string; resourceKey: string | null }> {
  // Try to get existing web view link
  const file = await drive.get<DriveFile>(`/files/${fileId}`, {
    fields: 'webViewLink,resourceKey'
  });

  if (file.webViewLink) {
    return {
      link: file.webViewLink,
      resourceKey: file.resourceKey ?? null
    };
  }

  // Create a public link if none exists
  const link = await createPublicLink(drive, fileId);
  return { link, resourceKey: await getResourceKey(drive, fileId) };
}

/**
 * Transfer ownership of a file to another user.
 * Note: The current owner must be the authenticated user.
 * @param drive DriveClient instance
 * @param fileId ID of the file
 * @param newOwnerEmail Email of the new owner
 */
export async function transferOwnership(
  drive: DriveClient,
  fileId: string,
  newOwnerEmail: string
): Promise<Permission> {
  return drive.post<Permission>(`/files/${fileId}/permissions`, {
    type: 'user',
    role: 'owner',
    emailAddress: newOwnerEmail
  }, {
    transferOwnership: 'true',
    sendNotificationEmail: 'true',
    fields: 'id,type,role,emailAddress'
  });
}
