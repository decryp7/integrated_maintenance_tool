// Test fixture for debrief-plugin-dotnet: known hot methods, allocations, exceptions, lock contention and
// waits. Prints "READY <pid>" once running; exits after the number of seconds given as the first
// argument (default 60) or when its standard input closes.
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Threading;

namespace Busy
{
    public sealed class Node
    {
        public Node Next;
        public byte[] Payload;
        public string Name;
    }

    public static class Cache
    {
        // A static root that keeps a linked list alive (heap snapshot: paths to roots, retained size).
        public static Node Head;
        public static readonly List<string> Names = new List<string>();
    }

    public static class Program
    {
        private static readonly object Gate = new object();
        private static volatile bool _stop;
        private static long _sink;

        public static int Main(string[] args)
        {
            int seconds = args.Length > 0 && int.TryParse(args[0], out var s) ? s : 60;
            BuildHeap();
            var threads = new List<Thread>
            {
                new Thread(SpinWorker) { Name = "spin", IsBackground = true },
                new Thread(AllocWorker) { Name = "alloc", IsBackground = true },
                new Thread(ThrowWorker) { Name = "throw", IsBackground = true },
                new Thread(LockWorker) { Name = "lock-a", IsBackground = true },
                new Thread(LockWorker) { Name = "lock-b", IsBackground = true },
                new Thread(SleepWorker) { Name = "sleep", IsBackground = true },
            };
            foreach (var t in threads) t.Start();
            Console.WriteLine("READY " + Process.GetCurrentProcess().Id);
            Console.Out.Flush();
            var stdin = new Thread(() => { try { Console.In.ReadToEnd(); } catch { } _stop = true; }) { IsBackground = true };
            stdin.Start();
            var watch = Stopwatch.StartNew();
            while (!_stop && watch.Elapsed.TotalSeconds < seconds) Thread.Sleep(50);
            _stop = true;
            return (int)(_sink & 1);
        }

        private static void BuildHeap()
        {
            Node head = null;
            for (int i = 0; i < 2000; i++)
            {
                head = new Node { Next = head, Payload = new byte[256], Name = "node-" + (i % 100) };
                Cache.Names.Add("name-" + (i % 50));
            }
            Cache.Head = head;
        }

        private static void SpinWorker()
        {
            while (!_stop) _sink += HotLoop(200000);
        }

        public static long HotLoop(int n)
        {
            long acc = 0;
            for (int i = 0; i < n; i++) acc += (i * 31) ^ (acc >> 3);
            return acc;
        }

        private static void AllocWorker()
        {
            var keep = new List<byte[]>();
            while (!_stop)
            {
                keep.Add(AllocateBuffer(1024));
                if (keep.Count > 1000) keep.Clear();
                if (keep.Count % 100 == 0) keep.Add(new byte[100000]);
            }
        }

        public static byte[] AllocateBuffer(int size) => new byte[size];

        private static void ThrowWorker()
        {
            while (!_stop)
            {
                try { Fail(); } catch (InvalidOperationException) { }
                Thread.Sleep(5);
            }
        }

        public static void Fail() => throw new InvalidOperationException("fixture");

        private static void LockWorker()
        {
            while (!_stop)
            {
                lock (Gate)
                {
                    _sink += HotLoop(50000);
                }
            }
        }

        private static void SleepWorker()
        {
            while (!_stop) Thread.Sleep(20);
        }
    }
}
