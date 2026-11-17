using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Threading;
using System.Windows.Automation;

public class UiHelper {
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT r);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hWnd, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint dwFlags, int dx, int dy, uint dwData, UIntPtr dwExtraInfo);
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint lpdwProcessId);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint idAttach, uint idAttachTo, bool fAttach);
    [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();

    public struct RECT { public int Left, Top, Right, Bottom; }

    public static void Click(int x, int y) {
        SetCursorPos(x, y);
        Thread.Sleep(200);
        mouse_event(0x0002, 0, 0, 0, UIntPtr.Zero);
        Thread.Sleep(60);
        mouse_event(0x0004, 0, 0, 0, UIntPtr.Zero);
        Thread.Sleep(150);
        mouse_event(0x0002, 0, 0, 0, UIntPtr.Zero);
        Thread.Sleep(60);
        mouse_event(0x0004, 0, 0, 0, UIntPtr.Zero);
        Thread.Sleep(500);
    }

    public static void ClickOnce(int x, int y) {
        SetCursorPos(x, y);
        Thread.Sleep(200);
        mouse_event(0x0002, 0, 0, 0, UIntPtr.Zero);
        Thread.Sleep(60);
        mouse_event(0x0004, 0, 0, 0, UIntPtr.Zero);
        Thread.Sleep(600);
    }

    public static void RightClick(int x, int y) {
        SetCursorPos(x, y);
        Thread.Sleep(200);
        mouse_event(0x0008, 0, 0, 0, UIntPtr.Zero); // RIGHTDOWN
        Thread.Sleep(60);
        mouse_event(0x0010, 0, 0, 0, UIntPtr.Zero); // RIGHTUP
        Thread.Sleep(700);
    }

    public static void ForceForeground(IntPtr hWnd) {
        uint dummy;
        uint foreThread = GetWindowThreadProcessId(GetForegroundWindow(), out dummy);
        uint appThread = GetCurrentThreadId();
        if (foreThread != appThread) AttachThreadInput(foreThread, appThread, true);
        BringWindowToTop(hWnd);
        ShowWindow(hWnd, 9);
        SetForegroundWindow(hWnd);
        if (foreThread != appThread) AttachThreadInput(foreThread, appThread, false);
        Thread.Sleep(300);
    }

    public static void Snap(IntPtr h, string path) {
        ForceForeground(h);
        Thread.Sleep(500);
        RECT r;
        GetWindowRect(h, out r);
        int w = r.Right - r.Left, hgt = r.Bottom - r.Top;
        if (w <= 0 || hgt <= 0) return;
        using (Bitmap bmp = new Bitmap(w, hgt))
        using (Graphics g = Graphics.FromImage(bmp)) {
            IntPtr hdc = g.GetHdc();
            PrintWindow(h, hdc, 2);
            g.ReleaseHdc(hdc);
            bmp.Save(path, ImageFormat.Png);
        }
        Console.WriteLine("Saved " + path + " (" + w + "x" + hgt + ")");
    }

    public static IntPtr FindProcWindow(int pid) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((hWnd, lParam) => {
            uint procId;
            GetWindowThreadProcessId(hWnd, out procId);
            if (procId == pid && IsWindowVisible(hWnd)) {
                RECT r;
                GetWindowRect(hWnd, out r);
                int w = r.Right - r.Left, h = r.Bottom - r.Top;
                if (w > 200 && h > 200) {
                    found = hWnd;
                    return false;
                }
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }

    static void Dump(AutomationElement el, int depth) {
        if (depth > 6) return;
        string indent = new string(' ', depth * 2);
        string name = el.Current.Name;
        string type = el.Current.ControlType.ProgrammaticName.Replace("ControlType.", "");
        System.Windows.Rect r = el.Current.BoundingRectangle;
        string rect = (int)r.X + "," + (int)r.Y + " " + (int)r.Width + "x" + (int)r.Height;
        Console.WriteLine(indent + type + " [" + rect + "] \"" + name + "\"");
        foreach (AutomationElement child in el.FindAll(TreeScope.Children, Condition.TrueCondition)) {
            Dump(child, depth + 1);
        }
    }

    public static void Main(string[] args) {
        SetProcessDPIAware();
        Console.OutputEncoding = System.Text.Encoding.UTF8;
        int pid = int.Parse(args[0]);
        string action = args[1];
        IntPtr h = FindProcWindow(pid);
        if (h == IntPtr.Zero) {
            Console.WriteLine("Window not found for PID " + pid);
            return;
        }

        RECT r;
        GetWindowRect(h, out r);

        if (action == "snap") {
            Snap(h, args[2]);
        } else if (action == "click_rel_and_snap") {
            int rx = int.Parse(args[2]);
            int ry = int.Parse(args[3]);
            string path = args[4];
            ForceForeground(h);
            Thread.Sleep(400);
            Console.WriteLine("Window rect: " + r.Left + "," + r.Top + " " + (r.Right-r.Left) + "x" + (r.Bottom-r.Top));
            Console.WriteLine("Clicking screen: (" + (r.Left + rx) + ", " + (r.Top + ry) + ")");
            Click(r.Left + rx, r.Top + ry);
            Snap(h, path);
        } else if (action == "type_and_snap") {
            // type text then Enter, then snap: args[2] = text, args[3] = path
            string text = args[2];
            string path = args[3];
            ForceForeground(h);
            Thread.Sleep(400);
            System.Windows.Forms.SendKeys.SendWait(text);
            Thread.Sleep(150);
            System.Windows.Forms.SendKeys.SendWait("{ENTER}");
            Thread.Sleep(700);
            Snap(h, path);
        } else if (action == "menu_hover") {
            // rclick x1 y1 -> hover x2 y2 -> snap path
            int x1 = int.Parse(args[2]), y1 = int.Parse(args[3]);
            int x2 = int.Parse(args[4]), y2 = int.Parse(args[5]);
            string path = args[6];
            ForceForeground(h);
            Thread.Sleep(400);
            RightClick(r.Left + x1, r.Top + y1);
            SetCursorPos(r.Left + x2, r.Top + y2);
            Thread.Sleep(900);
            Snap(h, path);
        } else if (action == "menu_flow") {
            // rclick x1 y1 -> hover x2 y2 -> click x3 y3 -> snap path
            int x1 = int.Parse(args[2]), y1 = int.Parse(args[3]);
            int x2 = int.Parse(args[4]), y2 = int.Parse(args[5]);
            int x3 = int.Parse(args[6]), y3 = int.Parse(args[7]);
            string path = args[8];
            ForceForeground(h);
            Thread.Sleep(400);
            RightClick(r.Left + x1, r.Top + y1);
            SetCursorPos(r.Left + x2, r.Top + y2);
            Thread.Sleep(900);
            ClickOnce(r.Left + x3, r.Top + y3);
            Snap(h, path);
        } else if (action == "move_rel_and_snap") {
            int rx = int.Parse(args[2]);
            int ry = int.Parse(args[3]);
            string path = args[4];
            ForceForeground(h);
            Thread.Sleep(400);
            SetCursorPos(r.Left + rx, r.Top + ry);
            Thread.Sleep(800);
            Snap(h, path);
        } else if (action == "click1_rel_and_snap") {
            int rx = int.Parse(args[2]);
            int ry = int.Parse(args[3]);
            string path = args[4];
            ForceForeground(h);
            Thread.Sleep(400);
            Console.WriteLine("Clicking screen: (" + (r.Left + rx) + ", " + (r.Top + ry) + ")");
            ClickOnce(r.Left + rx, r.Top + ry);
            Snap(h, path);
        } else if (action == "rclick_rel_and_snap") {
            int rx = int.Parse(args[2]);
            int ry = int.Parse(args[3]);
            string path = args[4];
            ForceForeground(h);
            Thread.Sleep(400);
            Console.WriteLine("Window rect: " + r.Left + "," + r.Top + " " + (r.Right-r.Left) + "x" + (r.Bottom-r.Top));
            Console.WriteLine("Right-clicking screen: (" + (r.Left + rx) + ", " + (r.Top + ry) + ")");
            RightClick(r.Left + rx, r.Top + ry);
            Snap(h, path);
        } else if (action == "click_abs_and_snap") {
            int ax = int.Parse(args[2]);
            int ay = int.Parse(args[3]);
            string path = args[4];
            ForceForeground(h);
            Thread.Sleep(400);
            Click(ax, ay);
            Snap(h, path);
        } else if (action == "dump") {
            AutomationElement root = AutomationElement.FromHandle(h);
            Dump(root, 0);
        }
    }
}
