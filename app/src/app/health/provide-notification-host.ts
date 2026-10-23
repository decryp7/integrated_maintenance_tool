import { ApplicationRef, createComponent, EnvironmentInjector, EnvironmentProviders, inject, makeEnvironmentProviders, provideAppInitializer, Provider, Type } from '@angular/core';
import { NOTIFICATION_SOURCES, NotificationSource } from './notification-host.model';
import { NotificationHost } from './notification-host';
import { ServiceHealthStore } from './service-health.store';

/** Registers a feature's notifications with the host (`useExisting`: the source is a root service). */
export function provideNotificationSource(source: Type<NotificationSource>): Provider {
  return { provide: NOTIFICATION_SOURCES, multi: true, useExisting: source };
}

/** Mounts the host once over the window and reads the first service-health snapshot. One line in the app config. */
export function provideNotificationHost(): EnvironmentProviders {
  return makeEnvironmentProviders([
    provideAppInitializer(() => {
      const host = document.createElement('debrief-notification-host');
      document.body.append(host);
      const ref = createComponent(NotificationHost, { environmentInjector: inject(EnvironmentInjector), hostElement: host });
      inject(ApplicationRef).attachView(ref.hostView);
      // A new window asks for the services' state at once: it must not assume everything is healthy.
      inject(ServiceHealthStore);
    }),
  ]);
}
