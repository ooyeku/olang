public class Fib {
    static long fib(long n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }
    public static void main(String[] a) {
        long t0 = System.nanoTime();
        long r = fib(32);
        long ms = (System.nanoTime() - t0) / 1000000;
        System.out.println("CHECK " + r);
        System.out.println("MS " + ms);
    }
}
