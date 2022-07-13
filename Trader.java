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
    private int currency;
    private int exchange;
    private String dataFilePath;
    private File dataFile;
    private ArrayList<Float> dataFilePrice;
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

    /**
     * Prints all lines in the dataFile
     */
    public void dataFilePrint() {
        /* Try catch block for printing dataFile */
        try {
            /* Scan through dataFileStream */
            Scanner dataFileStream = new Scanner(this.dataFile);
            while (dataFileStream.hasNext()) {
                String dataFileLine = dataFileStream.nextLine();
                System.out.println(dataFileLine);
            }
        } catch (FileNotFoundException e) {
            e.printStackTrace();
        }
    }

    /**
     * Parses column values of each line in the datafile into an ArrayList
     */
    public void dataFileParsePrice(String columnPriceName) {
        /* Processing variables */
        String[] dataFileLine;
        int columnIndexPrice = 0;
        int currentRow = 0;
        /* Define ArrayList */
        dataFilePrice = new ArrayList<Float>();
        try {
            /* Scan through dataFileStream */
            Scanner dataFileStream = new Scanner(this.dataFile);
            /* Parse the title line of the CSV */
            dataFileLine = dataFileStream.nextLine().split(",");
            for (int i = 0; i < dataFileLine.length; i++) {
                /* If the column matches parameters */
                if (dataFileLine[i].equals(columnPriceName)) {
                    columnIndexPrice = i;
                    break;
                } 
            }
            /* Parse all data lines of the CSV */
            while (dataFileStream.hasNext()) {
                dataFileLine = dataFileStream.nextLine().split(",");
                dataFilePrice.add(Float.parseFloat(dataFileLine[columnIndexPrice]));
                currentRow++;
            }
        } catch (FileNotFoundException e) {
            e.printStackTrace();
        }
    }


    public ArrayList<Integer> dataFileRecognizeBottoms() {
        return null;
    }

    public static void main(String[] args) {
        Trader trader = new Trader("Binance_BTCAUD_1h.csv", 10);
        trader.dataFileParsePrice("close");
        for(int i = 0; i < trader.dataFilePrice.size(); i++) {
            System.out.println(trader.dataFilePrice.get(i));
        }
    }
}
