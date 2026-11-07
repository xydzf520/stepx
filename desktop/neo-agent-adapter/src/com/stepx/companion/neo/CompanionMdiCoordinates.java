package com.stepx.companion.neo;
import java.util.regex.*;
/** Share one coordinate space between visible tree bounds and model actions. */
final class CompanionMdiCoordinates {
    static String normalizedEvidence(String value){return value.replaceAll("[\\s\\p{Z}]+","").trim();}
    static boolean visualEvidenceMatches(String evidence,String label){
        String a=normalizedEvidence(evidence),b=normalizedEvidence(label);
        return a.length()>=4&&a.equals(b);
    }
    private static final Pattern BOUNDS=Pattern.compile("\\[(-?\\d+),(-?\\d+)\\]\\[(-?\\d+),(-?\\d+)\\]");
    static int pixel(double n,int size){if(!Double.isFinite(n)||n<0||n>1000||size<=0)throw new IllegalArgumentException("坐标必须在 0 到 1000 之间");return Math.min(size-1,(int)Math.round(n*size/1000));}
    private static int unit(int n,int size){return Math.max(0,Math.min(1000,(int)Math.round(n*1000.0/size)));}
    /** Compare the hit path, not unrelated ads/timers elsewhere in the viewport. */
    static boolean stableTarget(String before,String after,double nx,double ny,int width,int height){
        int x=pixel(nx,width),y=pixel(ny,height);
        String a=hitPath(before,x,y),b=hitPath(after,x,y);
        return !a.isEmpty()&&a.equals(b);
    }
    private static String hitPath(String tree,int x,int y){
        java.util.List<String> hits=new java.util.ArrayList<>();
        for(String line:tree.split("\\n")){
            Matcher m=BOUNDS.matcher(line);if(!m.find())continue;
            int l=Integer.parseInt(m.group(1)),t=Integer.parseInt(m.group(2)),r=Integer.parseInt(m.group(3)),b=Integer.parseInt(m.group(4));
            if(x<l||x>=r||y<t||y>=b)continue;
            // Tree indentation and traversal numbers can change when an ad adds siblings.
            String identity=line.substring(0,m.start()).trim().replaceFirst("^[\\s\\d│├└─:#.\\-]+","");
            hits.add(identity+" "+m.group()+line.substring(m.end()).trim());
        }
        java.util.Collections.sort(hits);return String.join("\n",hits);
    }
    static String visibleTree(String tree,int width,int height){
        if(width<=0||height<=0)throw new IllegalArgumentException("截图尺寸无效");StringBuilder out=new StringBuilder();
        for(String line:tree.split("\\n")){
            Matcher m=BOUNDS.matcher(line);if(m.find()){
                int x=Integer.parseInt(m.group(1)),y=Integer.parseInt(m.group(2)),r=Integer.parseInt(m.group(3)),b=Integer.parseInt(m.group(4));
                if(r<=0||b<=0||x>=width||y>=height)continue;
                line=line.substring(0,m.start())+"["+unit(x,width)+","+unit(y,height)+"]["+unit(r,width)+","+unit(b,height)+"]"+line.substring(m.end());
            }out.append(line).append('\n');
        }return out.toString();
    }
}
