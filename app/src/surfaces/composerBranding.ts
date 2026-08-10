export interface ComposerBrandingCopy {
  ariaLabel: string;
  initialPlaceholder: string;
  projectPlaceholder: string;
  goalHelp: string;
  goalStatus: string;
  queuedTitle: string;
}

export function composerBrandingCopy(productName: string): ComposerBrandingCopy {
  return {
    ariaLabel: `Message ${productName}`,
    initialPlaceholder: `Describe what you want ${productName} to do…`,
    projectPlaceholder: `Ask ${productName} anything about this project…`,
    goalHelp: `Describe what ${productName} should keep working toward after /goal.`,
    goalStatus: `${productName} keeps going until it is done`,
    queuedTitle: `Queue message (sends when ${productName} finishes)`,
  };
}
