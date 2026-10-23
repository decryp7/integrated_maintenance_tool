import { SetupEvent } from '../ai.types';

/** A set-up or activation event that is still in progress (not its end). */
export type RunningStage = Exclude<SetupEvent, { stage: 'ready' } | { stage: 'failed' } | { stage: 'cancelled' } | { stage: 'fellBackToCpu' }>;

/** Plain words for a running stage. */
export function stageLabel(stage: RunningStage): string {
  switch (stage.stage) {
    case 'probing':
      return $localize`:@@ai.setup.stage.probing:Checking this computer…`;
    case 'recommending':
      return $localize`:@@ai.setup.stage.recommending:Choosing settings…`;
    case 'downloading':
      return $localize`:@@ai.setup.stage.downloading:Downloading the model…`;
    case 'verifying':
      return $localize`:@@ai.setup.stage.verifying:Verifying the download…`;
    case 'validating':
      return $localize`:@@ai.setup.stage.validating:Checking the model file…`;
    case 'loading':
      return stage.gpu ? $localize`:@@ai.setup.stage.loadingGpu:Loading the model on the GPU…` : $localize`:@@ai.setup.stage.loadingCpu:Loading the model on the CPU…`;
    case 'warmingUp':
      return $localize`:@@ai.setup.stage.warmingUp:Warming up…`;
  }
}
