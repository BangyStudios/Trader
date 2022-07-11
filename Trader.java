import java.io.File;
import java.io.FileNotFoundException;
import java.util.Scanner;
import java.util.ArrayList;

/* Hashdefines */
enum Currency{BTC};
enum Exchange{Coinbase};

/**
 * Trader class
 */
public class Trader {
    private int traderCurrency;
    private int traderExchange;
    private String dataFilePath;
    private File dataFile;
    private int threshold;

    /**
     * Constructor for Trader
     * @param dataFile Path to dataFile for testing different values
     * @param threshold Percentage threshold
     */
    public Trader(String dataFilePath, int threshold) {
        /* Load file descriptors */
        this.dataFilePath = dataFilePath; // Store dataFile path
        this.dataFile = new File(dataFilePath); // Open dataFile

        /* Store local variables to Trader */
        this.threshold = threshold;
    }

    public void dataFilePrint() {
        try {
            Scanner dataFileStream = new Scanner(this.dataFile);
            while (dataFileStream.hasNext()) {
                String dataFileLine = dataFileStream.nextLine();
                System.out.println(dataFileLine);
            }
        } catch (FileNotFoundException e) {
            e.printStackTrace();
        }
    }

    public static void main(String[] args) {
        Trader trader = new Trader("Binance_BTCAUD_1h.csv", 10);
        trader.dataFilePrint();
    }
}
