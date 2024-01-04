import { Outlet } from "react-router-dom";
import { getCurrent } from "@tauri-apps/api/window";
import { useTheme } from "@/services/model";
import { useEffect } from "react";

export const Layout = () => {
  const { setTheme } = useTheme((s) => s);

  useEffect(() => {
    setTheme("light");
  }, []);

  return (
    <div
      className="h-screen flex"
      onPointerDown={(e: any) => {
        if (e.target?.dataset?.windrag) getCurrent().startDragging();
      }}
    >
      <div className="flex-none w-[200px] border-r bg-secondary" data-windrag>
        Layout
      </div>

      <div className="flex-1">
        <Outlet />
      </div>
    </div>
  );
};
