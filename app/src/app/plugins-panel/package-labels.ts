import { InstalledState, PermissionDto, SignatureDto } from '../core/api.types';

/** A permission's short name (what it allows is the engine's `allows` text). */
export function permissionTitle(permission: PermissionDto): string {
  switch (permission) {
    case 'openedFilePaths':
      return $localize`:@@pluginsPanel.packages.permission.openedFilePaths:Read opened files`;
    case 'runPrograms':
      return $localize`:@@pluginsPanel.packages.permission.runPrograms:Run programs`;
    case 'writeFolders':
      return $localize`:@@pluginsPanel.packages.permission.writeFolders:Write to folders`;
    case 'network':
      return $localize`:@@pluginsPanel.packages.permission.network:Network access`;
    case 'readFolders':
      return $localize`:@@pluginsPanel.packages.permission.readFolders:Read folders`;
  }
}

/** An installed plugin's state as a badge. */
export function stateLabel(state: InstalledState): string {
  switch (state) {
    case 'ready':
      return $localize`:@@pluginsPanel.packages.state.ready:ready`;
    case 'running':
      return $localize`:@@pluginsPanel.packages.state.running:running`;
    case 'quarantined':
      return $localize`:@@pluginsPanel.packages.state.quarantined:quarantined`;
    case 'disabled':
      return $localize`:@@pluginsPanel.packages.state.disabled:disabled`;
    case 'pendingRestart':
      return $localize`:@@pluginsPanel.packages.state.pendingRestart:pending restart`;
    case 'unavailable':
      return $localize`:@@pluginsPanel.packages.state.unavailable:unavailable`;
  }
}

/** Who signed a package, in words. */
export function signatureText(signature: SignatureDto): string {
  switch (signature.kind) {
    case 'trusted':
      return $localize`:@@pluginsPanel.packages.signature.trusted:Signed by ${signature.signer ?? '?'}:signer:`;
    case 'unknownKey':
      return $localize`:@@pluginsPanel.packages.signature.unknownKey:Signed with an unknown key ${signature.signer ?? '?'}:key:`;
    case 'unsigned':
      return $localize`:@@pluginsPanel.packages.signature.unsigned:Not signed`;
  }
}

/** A refusal the user can lift by trusting the publisher's key (or, for their own builds, developer mode). */
export function refusalLiftedByTrust(reason: string): boolean {
  return reason === 'unsigned' || reason === 'unknownKey';
}
