// @ts-ignore
import logoImg from "../assets/logo.png";
import { APP_VERSION } from "../utils/constants";

interface AstroLogoProps {
    size?: number;
    showText?: boolean;
    className?: string;
}

export function AstroLogo({ size = 32, showText = true, className = "" }: AstroLogoProps) {
    return (
        <div className={`flex items-center gap-3 ${className}`}>
            <div className="relative">
                <div className="absolute inset-0 bg-blue-500/20 blur-lg rounded-full" />
                <img
                    src={logoImg}
                    alt="AstroBurst Logo"
                    style={{ width: size, height: size }}
                    className="relative z-10 object-contain"
                />
            </div>

            {showText && (
                <div className="flex flex-col">
          <span className="text-sm font-bold text-zinc-100 tracking-widest uppercase">
            AstroBurst
          </span>
                    <span className="text-[10px] text-blue-400/60 font-mono tracking-tighter">
            {APP_VERSION}
          </span>
                </div>
            )}
        </div>
    );
}
