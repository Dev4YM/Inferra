import { type ReactNode, useEffect, useRef, useState } from "react";

import { cn } from "@/lib/utils";

type ChartSize = {
  width: number;
  height: number;
};

export function ResponsiveChartFrame({
  className,
  children,
  fallback = null,
}: {
  className?: string;
  children: (size: ChartSize) => ReactNode;
  fallback?: ReactNode;
}) {
  const ref = useRef<HTMLDivElement | null>(null);
  const [size, setSize] = useState<ChartSize>({ width: 0, height: 0 });

  useEffect(() => {
    const element = ref.current;
    if (!element || typeof ResizeObserver === "undefined") return;

    const updateSize = (width: number, height: number) => {
      const nextWidth = Math.max(0, Math.floor(width));
      const nextHeight = Math.max(0, Math.floor(height));
      setSize((current) =>
        current.width === nextWidth && current.height === nextHeight
          ? current
          : { width: nextWidth, height: nextHeight },
      );
    };

    updateSize(element.clientWidth, element.clientHeight);

    const observer = new ResizeObserver((entries) => {
      const entry = entries[0];
      if (!entry) return;
      updateSize(entry.contentRect.width, entry.contentRect.height);
    });

    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  const ready = size.width > 0 && size.height > 0;

  return (
    <div ref={ref} className={cn("h-full min-w-0", className)}>
      {ready ? children(size) : fallback}
    </div>
  );
}
