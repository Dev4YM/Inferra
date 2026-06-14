import type { HTMLAttributes } from "react";
import { cva, type VariantProps } from "class-variance-authority";

import { cn } from "@/lib/utils";

const alertVariants = cva("flex items-start gap-3 rounded-md border px-3 py-2.5 text-sm [&>svg]:mt-0.5 [&>svg]:size-4 [&>svg]:shrink-0", {
  variants: {
    variant: {
      default: "border-border bg-card text-foreground",
      warning: "border-warning/50 bg-warning/8 text-foreground",
      destructive: "border-critical/50 bg-critical/8 text-foreground",
      success: "border-success/50 bg-success/8 text-foreground",
      info: "border-border bg-panel-inset text-foreground",
    },
  },
  defaultVariants: {
    variant: "default",
  },
});

export interface AlertProps extends HTMLAttributes<HTMLDivElement>, VariantProps<typeof alertVariants> {}

export function Alert({ className, variant, ...props }: AlertProps) {
  return <div className={cn(alertVariants({ variant }), className)} role="alert" {...props} />;
}

export function AlertTitle({ className, ...props }: HTMLAttributes<HTMLParagraphElement>) {
  return <p className={cn("font-semibold", className)} {...props} />;
}

export function AlertDescription({ className, ...props }: HTMLAttributes<HTMLParagraphElement>) {
  return <p className={cn("mt-1 leading-relaxed text-current/90", className)} {...props} />;
}
