import { RedactMode } from '../../core/api.types';

/** User-readable names of the built-in detectors; an id the UI does not know yet shows as itself. */
export function detectorLabel(id: string): string {
  switch (id) {
    case 'bearer-token':
      return $localize`:@@share.redactedCopy.detector.bearerToken:Bearer tokens`;
    case 'jwt':
      return $localize`:@@share.redactedCopy.detector.jwt:JSON web tokens (JWT)`;
    case 'api-key':
      return $localize`:@@share.redactedCopy.detector.apiKey:API keys and secrets`;
    case 'connection-string':
      return $localize`:@@share.redactedCopy.detector.connectionString:Connection string secrets`;
    case 'password':
      return $localize`:@@share.redactedCopy.detector.password:Passwords`;
    case 'user-path':
      return $localize`:@@share.redactedCopy.detector.userPath:User names in paths`;
    case 'email':
      return $localize`:@@share.redactedCopy.detector.email:Email addresses`;
    case 'credit-card':
      return $localize`:@@share.redactedCopy.detector.creditCard:Credit card numbers`;
    case 'ipv4':
      return $localize`:@@share.redactedCopy.detector.ipv4:IPv4 addresses`;
    case 'ipv6':
      return $localize`:@@share.redactedCopy.detector.ipv6:IPv6 addresses`;
    case 'guid':
      return $localize`:@@share.redactedCopy.detector.guid:GUIDs`;
    case 'phone':
      return $localize`:@@share.redactedCopy.detector.phone:Phone numbers`;
    case 'hostname':
      return $localize`:@@share.redactedCopy.detector.hostname:Host names`;
    default:
      return id;
  }
}

/** The radio choices for how a match is replaced, with a one-line explanation each. */
export function modeChoices(): { mode: RedactMode; label: string; hint: string }[] {
  return [
    { mode: 'mask', label: $localize`:@@share.redactedCopy.mode.mask:Mask`, hint: $localize`:@@share.redactedCopy.mode.maskHint:Replace each value with a label such as <email>.` },
    {
      mode: 'hash',
      label: $localize`:@@share.redactedCopy.mode.hash:Hash`,
      hint: $localize`:@@share.redactedCopy.mode.hashHint:Replace each value with a short code; equal values stay equal within the copy.`,
    },
    { mode: 'drop', label: $localize`:@@share.redactedCopy.mode.drop:Drop`, hint: $localize`:@@share.redactedCopy.mode.dropHint:Remove each value completely.` },
  ];
}
